//! `local_scope`：**本线程那条本地队列的别名**——投递点与驱动点。
//!
//! # 为什么本地队列是这个形状
//!
//! smol 2.x **没有**内建的 `spawn_local`：它只提供全局执行器（`smol::spawn`，
//! 进程级单例，见 `spawn_send.rs`）。本地队列由
//! `async_executor::LocalExecutor`（smol 重导出为 `smol::LocalExecutor`）提供，
//! 并且**必须自己驱动**（`LocalExecutor::run` / `tick`）。它有三条硬约束：
//!
//! 1. 本地任务必须投递到**某一条** `LocalExecutor` 上；
//! 2. 那条队列归 `LocalExecutor` 的持有者所有，**必须由持有者驱动**；
//! 3. `LocalExecutor` 内部是 `Rc`，因此是 `!Send` 的，绑定创建它的线程。
//!
//! 因此队列**不能**并进运行时值：运行时值是零大小、`Send + Sync` 的标记
//! （smol 没有可捕获的运行时把手，见 `lib.rs`），而队列是线程独占、必须由持有者
//! 驱动的——两者根本不是同一种生命周期。见 `abs_art::runtime` 模块文档。
//!
//! # 队列在 `thread_local!` 里，[`LocalScope`] 只是它的别名
//!
//! 本后端把 `LocalExecutor` 放进**本线程的 `thread_local!`**，于是作用域退化为那条
//! 队列的一个别名：
//!
//! - `Runtime::local_scope()` 在**同一线程上幂等**——调多少次都是同一条队列；
//! - [`Clone`] 只增加一个别名（`Rc` 克隆），**不是**新建一条队列；
//! - 它是 `!Send` 的：想在别的线程上投递，就到那条线程上另取一个作用域；
//! - 队列寿命 = 线程寿命：线程退出时执行器一并销毁，未完成的任务随之消失。
//!
//! 历史教训：曾经把执行器塞进 `JoinHandle`、在句柄被 poll 时顺带 tick 它，
//! 于是「本地任务能否推进」变成了「调用方有没有在 poll 句柄」的函数，`detach()`
//! 消费句柄还会连带销毁执行器、把任务当场取消。现在队列归**本线程**所有，句柄只持有
//! `smol::Task`，`detach()` 之后任务继续被驱动（见本模块单测与 `join_handle.rs`）。
//!
//! 取得路径只有一条：`Runtime<CAPS>::local_scope()`（要求 `CAPS` 含
//! [`SPAWN_LOCAL`](abs_art::SPAWN_LOCAL)）——作用域不会脱离运行时值凭空出现，
//! 构造入口**不公开**。

use alloc::rc::Rc;
use core::{fmt, future::Future};

use abs_art::TrLocalScope;

use crate::join_handle::JoinHandle;

std::thread_local! {
    /// 本线程的本地队列：**全线程唯一**，[`LocalScope`] 只是它的别名。
    ///
    /// 惰性初始化（首次访问时建一条），线程退出时随 TLS 销毁。销毁时执行器里
    /// 未完成的任务被取消，这正是「队列寿命 = 线程寿命」的具体含义。
    static LOCAL_QUEUE_: Rc<smol::LocalExecutor<'static>> = Rc::new(smol::LocalExecutor::new());
}

/// smol 后端的本地作用域：**本线程那条 `LocalExecutor` 的别名**。
///
/// # 线程独占
///
/// 它是 `!Send` 的（`Rc<LocalExecutor<'static>>`）：队列绑定本线程，`run_until`
/// 只能在**这条**线程上驱动它。需要在别的线程上投递本地任务时，在那边另取一个作用域
/// （`Runtime::local_scope()` 没有先决条件，任何线程都能调）。
///
/// # 与运行时值的关系
///
/// 运行时值（[`Runtime`](crate::Runtime)）只是**交出**这个别名，交出之后两者互不
/// 归属：值被 drop 不影响队列（它在 TLS 里），队列也不归值所有。
///
/// # 克隆与「再取一次」是同一件事
///
/// [`Clone`] 只增加别名；同一线程上再次调 `Runtime::local_scope()` 得到的也是
/// **同一条**队列——两者都**不会**新建队列。「同一线程多条 `LocalExecutor`」
/// 不再是本 crate 的使用方式（旧版本每条作用域各建一条，单测
/// `separate_scopes_have_separate_queues` 记录过那个语义，现已反转）。
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
/// assert_eq!(smol::block_on(scope.run_until(handle)).unwrap(), 5);
/// ```
pub struct LocalScope {
    /// 本线程那条本地队列的别名（来自 `thread_local!`）。
    local_: Rc<smol::LocalExecutor<'static>>,
}

impl LocalScope {
    /// 后端起内部构造：由 `Runtime::local_scope()` 交出。
    ///
    /// 刻意**不**公开：作用域只能从运行时值取得，这样它的来源与能力位声明
    /// （`Runtime<CAPS>::local_scope()` 要求 `SPAWN_LOCAL`）都是可追溯的。
    ///
    /// 队列本身来自本线程的 TLS——本函数**不**新建队列。
    pub(crate) fn for_current_thread_() -> Self {
        Self {
            local_: LOCAL_QUEUE_.with(Rc::clone),
        }
    }
}

impl Clone for LocalScope {
    /// 多加一个别名（`Rc` 克隆），不是新建队列。
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

    /// 把 `future` 投递到本线程的本地队列。
    ///
    /// 队列随**线程**存活，不随任务句柄存活——因此
    /// [`TrJoinHandle::detach`](abs_art::TrJoinHandle::detach) 之后任务仍会被
    /// 持续驱动，直到它自己结束（或线程退出）。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        self.local_.spawn(future).into()
    }

    /// 异步驱动本线程的本地队列直到 `future` 完成（`LocalExecutor::run`）。
    ///
    /// smol 没有环境运行时，因此返回的 future 只需放在任意能驱动 async-io 反应器的
    /// 位置 await（典型写法是 `smol::block_on(scope.run_until(fut))`，或嵌在另一个
    /// `run_until` 里）。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future,
    {
        self.local_.run(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端「本地作用域」的单元测试。

    use std::{cell::Cell, rc::Rc, time::Duration};

    use abs_art::{SPAWN_LOCAL, TrJoinHandle, TrLocalScope};

    use crate::Runtime;

    /// 在**独立线程**上运行用例体。
    ///
    /// 本地队列在本线程的 `thread_local!` 里，而 libtest 会复用线程
    /// （`--test-threads=1` 时更是同一个线程跑完所有用例），所以只有换线程才能保证
    /// 每个用例拿到一条干净的队列。用例体 panic 会经 `join` 传回，不影响判定。
    fn in_fresh_thread_<F>(f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::spawn(f).join().expect("用例线程 panic");
    }

    /// 目的：验证 `run_until` 会驱动本线程的本地队列，`!Send` 任务能跑完。
    ///
    /// 手段：经 `current().local_scope()` 取作用域，用 `smol::block_on` 驱动
    /// `scope.run_until(..)`，在其中投递一个捕获 `Rc<u32>` 的本地任务并 await 其句柄。
    ///
    /// 判定：取回 `6 * 7 == 42`；若队列没有被驱动，await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        in_fresh_thread_(|| {
            let scope = crate::current().local_scope();

            let out = smol::block_on(scope.run_until(async {
                let rc = Rc::new(6u32);
                scope.spawn_local(async move { *rc * 7 }).await.unwrap()
            }));

            assert_eq!(out, 42);
        });
    }

    /// 目的：验证 `run_until` 可以由**外部驱动源**阻塞驱动（`smol::block_on` 包住它），
    /// 且等待期间本地队列持续推进。
    ///
    /// 手段：直接用 `smol::block_on(scope.run_until(..))`，在其中投递一个捕获 `Rc<u32>`
    /// 的 `!Send` 任务并 await 其句柄。
    ///
    /// 判定：取回 `6 * 7 == 42`；若 `run_until` 漏掉了本地队列（例如退化成直接返回
    /// `future`），await 会永久挂起。
    #[test]
    fn run_until_under_an_external_blocking_driver() {
        in_fresh_thread_(|| {
            let scope = crate::current().local_scope();

            let out = smol::block_on(scope.run_until(async {
                let rc = Rc::new(6u32);
                scope.spawn_local(async move { *rc * 7 }).await.unwrap()
            }));

            assert_eq!(out, 42);
        });
    }

    /// 目的：验证 `run_until` **可以嵌套**——外层与内层驱动的是同一条队列。
    ///
    /// 手段：在 `scope.run_until(..)` 内部再套一层 `scope.run_until(..)`，在最内层投递
    /// 一个 `!Send` 任务并 await 其句柄。
    ///
    /// 判定：取回 42。若 `LocalExecutor::run` 的重入会 panic 或内层不再 tick 队列，
    /// 本用例会 panic / 挂起——它把「嵌套是否成立」变成可执行事实，供 trait 文档引用。
    #[test]
    fn nested_run_until_drives_the_same_queue() {
        in_fresh_thread_(|| {
            let scope = crate::current().local_scope();

            let out = smol::block_on(scope.run_until(async {
                scope
                    .run_until(async {
                        let rc = Rc::new(6u32);
                        scope.spawn_local(async move { *rc * 7 }).await.unwrap()
                    })
                    .await
            }));

            assert_eq!(out, 42);
        });
    }

    /// 目的：验证本地队列归**本线程**所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 手段：在 `scope.run_until` 中投递一个置位 `Rc<Cell<bool>>` 的本地任务后
    /// 立即 `detach()`，再循环 `yield_now` 等标志置位。
    ///
    /// 判定：标志在有限次让出内被置位；若实现把队列绑在句柄上（detach 即销毁
    /// 执行器 → 任务取消），标志永远不会置位，循环会因超出上限而断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        in_fresh_thread_(|| {
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
        });
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
        in_fresh_thread_(|| {
            let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();

            let out = smol::block_on(scope.run_until(async {
                let rc = Rc::new(6u32);
                scope.spawn_local(async move { *rc * 7 }).await.unwrap()
            }));

            assert_eq!(out, 42);
        });
    }

    /// 目的：验证同一个作用域的克隆**共享同一条**本地队列。
    ///
    /// 手段：克隆作用域，用克隆体投递任务，用原作用域驱动，再 await 句柄。
    ///
    /// 判定：取回 5——若两个克隆各有各的队列，驱动原作用域不会推进克隆体投递的
    /// 任务，await 会挂起。
    #[test]
    fn clones_share_one_local_queue() {
        in_fresh_thread_(|| {
            let scope = crate::current().local_scope();
            let clone = scope.clone();
            let handle = clone.spawn_local(async { 5u32 });
            let out = smol::block_on(scope.run_until(handle)).unwrap();

            assert_eq!(out, 5);
        });
    }

    /// 目的：验证**本线程上多次 `local_scope()` 是同一条队列**（幂等的别名语义）。
    ///
    /// 手段：取两个作用域 A、B（**不**做 clone），用 B 投递、用 A 驱动。
    ///
    /// 判定：取回 5。旧版本每条 `local_scope()` 会新建一条 `LocalExecutor`，那时本
    /// 用例会挂起（`separate_scopes_have_separate_queues` 记录的是旧语义）；本次改造
    /// 把语义反转为「同一条」，本用例即该反转的回归闸门。
    #[test]
    fn separate_calls_share_the_thread_local_queue() {
        in_fresh_thread_(|| {
            let a = crate::current().local_scope();
            let b = crate::current().local_scope();

            let handle = b.spawn_local(async { 5u32 });
            let out = smol::block_on(a.run_until(handle)).unwrap();

            assert_eq!(out, 5);
        });
    }

    /// 目的：验证本地投递与**运行时值**的计时能力各自挂在正确的宿主上（本线程队列管
    /// 调度、运行时值管计时）。
    ///
    /// 手段：取运行时值 `value` 与它的作用域；在 `scope.run_until` 内 await
    /// `value.delay(1ms)`，量墙上耗时。
    ///
    /// 判定：耗时 ≥ 1ms；编译通过本身也证明作用域上没有 `delay`（计时 trait 实现
    /// 在 `Runtime` 上，`LocalScope` 不实现 `TrDelay`）。
    #[cfg(feature = "delay")]
    #[test]
    fn delay_comes_from_the_runtime_not_the_scope() {
        use abs_art::TrDelay;

        in_fresh_thread_(|| {
            let value = crate::current();
            let scope = value.local_scope();

            let started = std::time::Instant::now();
            smol::block_on(scope.run_until(async {
                value.delay(Duration::from_millis(1)).await;
            }));
            let elapsed = started.elapsed();

            assert!(elapsed >= Duration::from_millis(1), "耗时为 {elapsed:?}");
        });
    }

    /// 目的：验证 `TrLocalScope::Handle` 与 `crate::JoinHandle` 是同一个类型。
    ///
    /// 手段：把 `spawn_local` 交回的句柄直接传给一个形参类型为
    /// `crate::JoinHandle<u32>` 的函数。
    ///
    /// 判定：编译通过即为通过（类型相等）。
    #[test]
    fn handle_type_is_the_shared_join_handle() {
        in_fresh_thread_(|| {
            fn take_handle_(_: crate::JoinHandle<u32>) {}

            let scope = crate::current().local_scope();
            take_handle_(scope.spawn_local(async { 1u32 }));
        });
    }
}

#[cfg(feature = "mock-clock")]
impl LocalScope {
    /// 用**手动时钟**驱动本作用域：队列照常被驱动，而时间由测试自己推进。
    ///
    /// tick 钩子用 `LocalExecutor::try_tick()`（返回「本轮有没有跑到任务」）——这正是
    /// 「空闲即推进」的判据。
    ///
    /// # 它**不属于** [`TrLocalScope`](abs_art::TrLocalScope)
    ///
    /// 这是本后端在 `mock-clock` feature 下提供的**固有方法**，不是作用域契约的一部分
    /// ——抽象层的作用域只有 [`spawn_local`](abs_art::TrLocalScope::spawn_local) 与
    /// [`run_until`](abs_art::TrLocalScope::run_until) 两条。名字里的 `block_on` 只描述
    /// 它自己做的那次阻塞。
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

    use abs_art::{TrClock, TrDelay, TrJoinHandle, TrLocalScope};
    use abs_art_mock_clock::{ManualClock, ManualTime, MockInstant};

    /// 在独立线程上运行（理由见 `tests::in_fresh_thread_`）。
    fn in_fresh_thread_<F>(f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::spawn(f).join().expect("用例线程 panic");
    }

    /// 目的：验证「虚拟一小时」在真实时间里几乎瞬间完成。
    ///
    /// 手段：smol 无环境运行时前提，直接取值与作用域，`block_on_advancing` 跑 `delay(1h)`。
    ///
    /// 通过依据：真实耗时 < 1s，虚拟时刻恰为 3600_000ms。
    #[test]
    fn virtual_hour_passes_instantly() {
        in_fresh_thread_(|| {
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
        });
    }

    /// 目的：验证**投递到本地队列的任务**同样跑在虚拟时间上。
    ///
    /// 手段：`spawn_local` 一个睡虚拟 30 分钟的任务，主体 await 其句柄。
    ///
    /// 通过依据：取回 42，虚拟时刻推进到 1800_000ms，真实耗时 < 1s。
    #[test]
    fn spawned_local_task_runs_on_virtual_time() {
        in_fresh_thread_(|| {
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
        });
    }

    /// 让出一次执行权：自唤醒一次后返回 `Pending`。
    ///
    /// `wake_by_ref` 不可省——用于本文件里的「永远有活」任务时，它正是「每轮都重新
    /// 就绪」的来源；缺了它任务会真的 park 下去，执行器随即变成「没活」。
    async fn yield_once_() {
        let mut first = true;
        core::future::poll_fn(move |cx| {
            if first {
                first = false;
                cx.waker().wake_by_ref();
                core::task::Poll::Pending
            } else {
                core::task::Poll::Ready(())
            }
        })
        .await;
    }

    /// 目标契约：**执行器还有活时不得消耗虚拟时间**。
    ///
    /// - 手段：放一个「每轮自唤醒」的本地任务（执行器因此始终有就绪工作），再让主体
    ///   作 6 轮让出。
    /// - 判断：这 6 轮之间的虚拟时刻推进量必须是 0。跨三端的最小复现与病因见
    ///   `smux_v1/dev-notes/timer-mock-clock-and-generic-drop-20261006-1625.md` §11。
    #[test]
    fn busy_executor_does_not_advance_virtual_time() {
        in_fresh_thread_(|| {
            let value = crate::current();
            let scope = value.local_scope();
            let clock = ManualClock::new();
            let timed = ManualTime::new(value, clock.clone());
            let scope_spawn = scope.clone();
            let task_timed = timed.clone();
            let (start, end) = scope.block_on_advancing(&clock, async move {
                // 周期定时器：保证时钟「有下一个到期时刻」可推。
                scope_spawn
                    .spawn_local(async move {
                        loop {
                            task_timed.delay(Duration::from_millis(500u64)).await;
                        }
                    })
                    .detach();
                // 【契约】再放一个「永远有活」的任务（每轮自唤醒）：执行器始终有
                // 就绪工作，因此**不得**推进虚拟时钟。
                scope_spawn
                    .spawn_local(async move {
                        loop {
                            yield_once_().await;
                        }
                    })
                    .detach();
                for _ in 0..2 {
                    yield_once_().await;
                }
                let start = timed.now();
                for _ in 0..6 {
                    yield_once_().await;
                }
                let end = timed.now();
                (start, end)
            });
            assert_eq!(
                (end - start).as_millis(),
                0u128,
                "执行器有活时不该消耗虚拟时间"
            );
        });
    }
}
