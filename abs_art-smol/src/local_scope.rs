//! `local_scope`：本地队列的**持有者与驱动点**——一个线程独占的作用域值。
//!
//! # 为什么本地队列是这个形状
//!
//! smol 2.x **没有**内建的 `spawn_local`：它只提供全局执行器（`smol::spawn`，
//! 进程级单例，见 `spawn_send.rs`）。本地队列只能由使用方用
//! `async_executor::LocalExecutor`（smol 重导出为 `smol::LocalExecutor`）自建，
//! 并且**自己驱动**（`LocalExecutor::run` / `tick`）。它有三条硬约束：
//!
//! 1. 本地任务必须投递到**某一条** `LocalExecutor` 上；
//! 2. 那条队列归 `LocalExecutor` 的持有者所有，**必须由持有者驱动**；
//! 3. `LocalExecutor` 内部是 `Rc`，因此是 `!Send` 的，绑定创建它的线程。
//!
//! 因此队列**不能**并进运行时值：运行时值是零大小、`Send + Sync` 的标记
//! （smol 没有可捕获的运行时把手，见 `lib.rs`），而队列是线程独占、必须由持有者
//! 驱动的——两者根本不是同一种生命周期。见 `abs_art::runtime` 模块文档。
//!
//! 本模块提供 [`LocalScope`]：它就是那条队列的持有者，同时是投递点与驱动点。
//!
//! - 投递走 `LocalExecutor::spawn`——它在队列未被驱动时也能投递且不 panic，正是
//!   「先建队列、后驱动」这个用法需要的语义；
//! - 驱动走 [`TrLocalScope::run_until`]（异步）或 [`TrLocalScope::block_on`]（阻塞）；
//! - 克隆作用域即共享**同一条**队列（`Rc<LocalExecutor<'static>>`）。
//!
//! 历史教训：曾经把执行器塞进 `JoinHandle`、在句柄被 poll 时顺带 tick 它，
//! 于是「本地任务能否推进」变成了「调用方有没有在 poll 句柄」的函数，`detach()`
//! 消费句柄还会连带销毁执行器、把任务当场取消。现在队列归作用域所有，句柄只持有
//! `smol::Task`，`detach()` 之后任务继续被驱动（见本模块单测与 `join_handle.rs`）。
//!
//! 取得路径只有一条：`Runtime<CAPS>::local_scope()`（要求 `CAPS` 含
//! [`SPAWN_LOCAL`](abs_art::SPAWN_LOCAL)）——作用域不会脱离运行时值凭空出现，
//! 构造函数**不公开**。

use alloc::rc::Rc;
use core::{fmt, future::Future};

use abs_art::TrLocalScope;

use crate::join_handle::JoinHandle;

/// smol 后端的本地作用域：持有一条**调用方创建、调用方驱动**的本地队列。
///
/// # 线程独占
///
/// 它是 `!Send` 的（`Rc<LocalExecutor<'static>>`）：队列绑定创建它的线程，只能在
/// **这条**线程上被驱动。需要在别的线程上投递本地任务时，在那边另取一个作用域
/// （`Runtime::local_scope()` 没有先决条件，任何线程都能调）。
///
/// # 与运行时值的关系
///
/// 运行时值（[`Runtime`](crate::Runtime)）只是**交出**本作用域，交出之后两者互不
/// 归属：值被 drop 不影响作用域（`Rc` 自持），作用域被 drop 也不影响值。因此
/// 「队列随作用域走」——同一进程里可以同时存在多条互不干扰的本地队列
/// （见本模块单测 `separate_scopes_have_separate_queues`）。
///
/// # 克隆
///
/// [`Clone`] 共享**同一条**队列：克隆出来的两个作用域是一条队列的两个把手，不是
/// 两条队列。要另一条独立队列，再调一次 `Runtime::local_scope()`。
///
/// # Examples
///
/// ```
/// use abs_art::{SPAWN_LOCAL, TrLocalScope};
/// use abs_art_smol::Runtime;
///
/// let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();
/// let clone = scope.clone();
///
/// // 克隆体投递，原作用域驱动：二者是**同一条**队列
/// let handle = clone.spawn_local(async { 5u32 });
/// assert_eq!(scope.block_on(handle).unwrap(), 5);
/// ```
pub struct LocalScope {
    /// 本作用域的本地队列（`Rc` 使克隆共享同一条队列，也使作用域 `!Send`）。
    local_: Rc<smol::LocalExecutor<'static>>,
}

impl LocalScope {
    /// 后端起内部构造：新建一条本地队列，由 `Runtime::local_scope()` 交出。
    ///
    /// 刻意**不**公开：作用域只能从运行时值取得，这样它的来源与能力位声明
    /// （`Runtime<CAPS>::local_scope()` 要求 `SPAWN_LOCAL`）都是可追溯的。
    pub(crate) fn with_executor() -> Self {
        Self {
            local_: Rc::new(smol::LocalExecutor::new()),
        }
    }
}

impl Clone for LocalScope {
    /// 共享**同一条**本地队列（`Rc` 克隆）。
    fn clone(&self) -> Self {
        Self {
            local_: Rc::clone(&self.local_),
        }
    }
}

impl fmt::Debug for LocalScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("abs_art_smol::LocalScope")
            .finish_non_exhaustive()
    }
}

impl TrLocalScope for LocalScope {
    /// 与全局 `spawn` / `spawn_blocking` 共用同一个 [`JoinHandle`]（它只是
    /// `smol::Task` 的薄包装，**不持有队列**）。
    type Handle<T>
        = JoinHandle<T>
    where
        T: 'static;

    /// 把 `future` 投递到本作用域的本地队列。
    ///
    /// 队列随作用域存活，**不随任务句柄存活**——因此
    /// [`TrJoinHandle::detach`](abs_art::TrJoinHandle::detach) 之后任务仍会被
    /// 持续驱动，直到它自己结束。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        self.local_.spawn(future).into()
    }

    /// 异步驱动本作用域的本地队列直到 `future` 完成。
    ///
    /// smol 没有环境运行时，因此返回的 future 只需放在任意能驱动 async-io 反应器的
    /// 位置 await（典型写法是 `smol::block_on(scope.run_until(fut))`，或嵌在另一个
    /// `run_until` / [`TrLocalScope::block_on`] 里）。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future,
    {
        self.local_.run(future)
    }

    /// 阻塞当前线程，驱动本作用域的本地队列直到 `future` 完成。
    ///
    /// 实现即 `smol::block_on(self.local_.run(future))`：等待期间 `LocalExecutor::run`
    /// 持续推进队列，因此这是「阻塞等待并驱动本地队列」的入口。
    ///
    /// # Panics
    ///
    /// 不 panic（smol 的 `block_on` 没有环境运行时前提）。
    fn block_on<F>(&self, future: F) -> <F as Future>::Output
    where
        F: Future,
    {
        smol::block_on(self.local_.run(future))
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端「本地作用域」的单元测试。

    use std::{cell::Cell, rc::Rc, time::Duration};

    use abs_art::{SPAWN_LOCAL, TrJoinHandle, TrLocalScope};

    use crate::Runtime;

    /// 目的：验证 `run_until` 会驱动本作用域的本地队列，`!Send` 任务能跑完。
    ///
    /// 手段：经 `current().local_scope()` 取作用域，用 `smol::block_on` 驱动
    /// `scope.run_until(..)`，在其中投递一个捕获 `Rc<u32>` 的本地任务并 await 其句柄。
    ///
    /// 判定：取回 `6 * 7 == 42`；若队列没有被驱动，await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let scope = crate::current().local_scope();

        let out = smol::block_on(scope.run_until(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证**阻塞**驱动入口 `TrLocalScope::block_on` 会驱动本作用域的本地
    /// 队列（`smol::block_on(local.run(fut))` 的形状）。
    ///
    /// 手段：直接调用 `scope.block_on(..)`，在其中投递一个捕获 `Rc<u32>` 的 `!Send`
    /// 任务并 await 其句柄。
    ///
    /// 判定：取回 `6 * 7 == 42`；若 `block_on` 漏掉了本地队列（例如退化成
    /// `smol::block_on(fut)`），await 会永久挂起。
    #[test]
    fn block_on_drives_local_tasks() {
        let scope = crate::current().local_scope();

        let out = scope.block_on(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归**作用域**所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 手段：在 `scope.run_until` 中投递一个置位 `Rc<Cell<bool>>` 的本地任务后
    /// 立即 `detach()`，再循环 `yield_now` 等标志置位。
    ///
    /// 判定：标志在有限次让出内被置位；若实现把队列绑在句柄上（detach 即销毁
    /// 执行器 → 任务取消），标志永远不会置位，循环会因超出上限而断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        let scope = crate::current().local_scope();

        smol::block_on(scope.run_until(async {
            let flag = Rc::new(Cell::new(false));
            let task_flag = flag.clone();

            let handle = scope.spawn_local(async move {
                smol::future::yield_now().await;
                task_flag.set(true);
            });
            handle.detach();

            let mut spins = 0u32;
            while !flag.get() {
                smol::future::yield_now().await;
                spins += 1;
                assert!(spins < 1_000_000, "detach 后本地任务未被推进");
            }
        }));
    }

    /// 目的：验证「声明 → 取得」路径——只写 `SPAWN_LOCAL` 的运行时值确实交得出
    /// 作用域，且作用域能承载 `!Send` 任务。
    ///
    /// 手段：把 CAPS 写成只含 `SPAWN_LOCAL`，经 `local_scope()` 取作用域并跑一个
    /// 捕获 `Rc` 的 `!Send` 任务。
    ///
    /// 判定：取回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错（例如标在别的位上），
    /// 本测试将无法编译。
    #[test]
    fn declared_cap_gives_usable_local_queue() {
        let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();

        let out = smol::block_on(scope.run_until(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证同一个作用域的克隆共享**同一条**本地队列。
    ///
    /// 手段：克隆作用域，用克隆体投递任务，用原作用域驱动，再 await 句柄。
    ///
    /// 判定：取回 5——若两个克隆各有各的队列，驱动原作用域不会推进克隆体投递的
    /// 任务，await 会挂起。
    #[test]
    fn clones_share_one_local_queue() {
        let scope = crate::current().local_scope();
        let clone = scope.clone();
        let handle = clone.spawn_local(async { 5u32 });
        let out = smol::block_on(scope.run_until(handle)).unwrap();

        assert_eq!(out, 5);
    }

    /// 目的：验证两条**独立**作用域各有各的队列——队列是调用方创建的独立对象，
    /// 驱动 B 不会推进投在 A 上的任务（`local_scope()` 每次调用给一条新队列）。
    ///
    /// 手段：造两个作用域 A、B；在 A 上投递一个置位 `Rc<Cell<bool>>` 的任务并
    /// `detach()`（避免句柄 drop 取消任务）；先只驱动 B（跑若干次 `yield_now`），
    /// 断言标志仍为 false；再驱动 A，等标志置位。
    ///
    /// 判定：驱动 B 之后标志为 false（若两条作用域共享队列，这里就会变 true 而
    /// 断言失败），驱动 A 之后标志为 true（若 A 的队列没被驱动，循环会因超出上限
    /// 而断言失败）。
    #[test]
    fn separate_scopes_have_separate_queues() {
        let scope_a = crate::current().local_scope();
        let scope_b = crate::current().local_scope();

        let flag = Rc::new(Cell::new(false));
        let task_flag = flag.clone();
        let handle = scope_a.spawn_local(async move {
            smol::future::yield_now().await;
            task_flag.set(true);
        });
        handle.detach();

        // 只驱动 B：A 上的任务不该有任何推进。
        scope_b.block_on(async {
            for _ in 0..16 {
                smol::future::yield_now().await;
            }
        });
        assert!(!flag.get(), "驱动 B 不该推进 A 的队列");

        // 驱动 A：任务跑完。
        let mut spins = 0u32;
        scope_a.block_on(async {
            while !flag.get() {
                smol::future::yield_now().await;
                spins += 1;
                assert!(spins < 1_000_000, "A 的队列未被推进");
            }
        });
        assert!(flag.get());
    }

    /// 目的：验证本地投递与**运行时值**的计时能力各自挂在正确的宿主上（作用域管
    /// 队列、运行时值管计时）。
    ///
    /// 手段：取运行时值 `value` 与它的作用域；在 `scope.block_on` 内 await
    /// `value.delay(1ms)`，量墙上耗时。
    ///
    /// 判定：耗时 ≥ 1ms；编译通过本身也证明作用域上没有 `delay`（计时 trait 实现
    /// 在 `Runtime` 上，`LocalScope` 不实现 `TrDelay`）。
    #[cfg(feature = "delay")]
    #[test]
    fn delay_comes_from_the_runtime_not_the_scope() {
        use abs_art::TrDelay;

        let value = crate::current();
        let scope = value.local_scope();

        let started = std::time::Instant::now();
        scope.block_on(async {
            value.delay(Duration::from_millis(1)).await;
        });
        let elapsed = started.elapsed();

        assert!(elapsed >= Duration::from_millis(1), "耗时为 {elapsed:?}");
    }

    /// 目的：验证 `TrLocalScope::Handle` 与 `crate::JoinHandle` 是同一个类型。
    ///
    /// 手段：把 `spawn_local` 交回的句柄直接传给一个形参类型为
    /// `crate::JoinHandle<u32>` 的函数。
    ///
    /// 判定：编译通过即为通过（类型相等）。
    #[test]
    fn handle_type_is_the_shared_join_handle() {
        fn take_handle_(_: crate::JoinHandle<u32>) {}

        let scope = crate::current().local_scope();
        take_handle_(scope.spawn_local(async { 1u32 }));
    }
}

#[cfg(feature = "mock-clock")]
impl LocalScope {
    /// 用**手动时钟**驱动本作用域：队列照常被驱动，而时间由测试自己推进。
    ///
    /// tick 钩子用 `LocalExecutor::try_tick()`（返回「本轮有没有跑到任务」）——这正是
    /// 「空闲即推进」的判据。
    ///
    /// # Panics
    ///
    /// 主体既没有就绪任务、也没有可推进的定时器时 panic（把静默挂起变成响亮失败）。
    pub fn block_on_advancing<F, C>(&self, clock: &C, body: F) -> <F as Future>::Output
    where
        F: Future,
        C: abs_art_mock_clock::ManualClockApi,
    {
        let ticker = Rc::clone(&self.local_);
        smol::block_on(abs_art_mock_clock::Supervisor::new(
            body,
            clock.clone(),
            move || ticker.try_tick(),
        ))
    }
}

#[cfg(all(test, feature = "mock-clock"))]
mod mock_clock_tests_ {
    //! smol 后端接上手动时钟之后的虚拟时间行为。

    use std::time::Duration;

    use abs_art::{TrClock, TrDelay, TrLocalScope};
    use abs_art_mock_clock::{ManualClock, ManualTime, MockInstant};

    /// 目的：验证「虚拟一小时」在真实时间里几乎瞬间完成。
    ///
    /// 手段：smol 无环境运行时前提，直接取值与作用域，`block_on_advancing` 跑 `delay(1h)`。
    ///
    /// 通过依据：真实耗时 < 1s，虚拟时刻恰为 3600_000ms。
    #[test]
    fn virtual_hour_passes_instantly() {
        let value = crate::current();
        let scope = value.local_scope();

        let clock = ManualClock::new();
        let timed = ManualTime::new(value, clock.clone());

        let started = std::time::Instant::now();
        let virtual_now = scope.block_on_advancing(&clock, async {
            timed.delay(Duration::from_secs(3_600)).await;
            timed.now()
        });
        let real = started.elapsed();

        assert!(real < Duration::from_secs(1), "真实耗时 {real:?}");
        assert_eq!(virtual_now.as_millis(), 3_600_000);
    }

    /// 目的：验证**投递到本地队列的任务**同样跑在虚拟时间上。
    ///
    /// 手段：`spawn_local` 一个睡虚拟 30 分钟的任务，主体 await 其句柄。
    ///
    /// 通过依据：取回 42，虚拟时刻推进到 1800_000ms，真实耗时 < 1s。
    #[test]
    fn spawned_local_task_runs_on_virtual_time() {
        let value = crate::current();
        let scope = value.local_scope();

        let clock = ManualClock::new();
        let scope_in_body = scope.clone();
        let started = std::time::Instant::now();
        let out = scope.block_on_advancing(&clock, {
            let clock = clock.clone();
            async move {
                let handle = scope_in_body.spawn_local({
                    let clock = clock.clone();
                    async move {
                        let timed = ManualTime::new(value, clock);
                        timed.delay(Duration::from_secs(1_800)).await;
                        42u32
                    }
                });
                handle.await.unwrap()
            }
        });
        let real = started.elapsed();

        assert_eq!(out, 42);
        assert_eq!(clock.now().as_millis(), 1_800_000);
        assert!(real < Duration::from_secs(1), "真实耗时 {real:?}");
    }
}
