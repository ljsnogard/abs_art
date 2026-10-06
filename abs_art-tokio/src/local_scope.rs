//! `local_scope`：**本线程那条本地队列的别名**——投递点与驱动点。
//!
//! # 为什么本地队列是这个形状
//!
//! tokio 的 `spawn_local` 有三条硬约束：
//!
//! 1. 本地任务必须投递到**某一条** `LocalSet` 上；
//! 2. 本地队列归 `LocalSet` 的持有者所有，**必须由持有者驱动**；
//! 3. `LocalSet` 是 `!Send` 的，绑定创建它的线程。
//!
//! 因此队列**不能**并进运行时值：运行时把手（`Handle`）是可跨线程共享、可长期活着
//! 的，而队列是线程独占、必须由持有者驱动的——两种生命周期完全不同。曾经合并过一次，
//! 代价是 tokio 的运行时值被迫 `!Send`（详见 `abs_art::runtime` 模块文档）。
//!
//! # 队列在 `thread_local!` 里，[`LocalScope`] 只是它的别名
//!
//! 本后端把 `LocalSet` 放进**本线程的 `thread_local!`**，于是作用域退化为那条队列的
//! 一个别名：
//!
//! - `Runtime::local_scope()` 在**同一线程上幂等**——调多少次都是同一条队列；
//! - [`Clone`] 只增加一个别名（`Rc` 克隆），**不是**新建一条队列；
//! - 它是 `!Send` 的：想在别的线程上投递，就到那条线程上另取一个作用域；
//! - 队列寿命 = 线程寿命：线程退出时 `LocalSet` 一并销毁，未完成的任务随之消失。
//!
//! 「同一线程多条 `LocalSet`」这套 tokio 用法被**显式排除**：家族只对齐 compio 的
//! 语义（见 [`TrLocalScope`]）。tokio 允许多个 `LocalSet`
//! 并存这件事在本后端不暴露——`local_scope()` 永远交出同一条。
//!
//! # 只提供 `run_until`，没有阻塞入口
//!
//! tokio 的 `block_in_place` 在 `LocalSet` 内被 tokio 自己禁止（`worker.rs` 的注释：
//! 「in a LocalSet, where it is _not_ okay to block」），所以「阻塞等待并驱动本地
//! 队列」在本后端根本立不住，抽象层因此把阻塞入口从 [`TrLocalScope`] 上删掉了。
//! 需要阻塞等待时用运行时值的
//! [`TrBlockOn::block_on`](abs_art::TrBlockOn::block_on)——但它**不驱动本地队列**。
//!
//! 取得路径只有一条：`Runtime<CAPS>::local_scope()`（要求 `CAPS` 含
//! [`SPAWN_LOCAL`](abs_art::SPAWN_LOCAL)）——作用域不会脱离运行时凭空出现。

use alloc::rc::Rc;
use core::{fmt, future::Future};

use abs_art::TrLocalScope;

use crate::JoinHandle;

std::thread_local! {
    /// 本线程的本地队列：**全线程唯一**，[`LocalScope`] 只是它的别名。
    ///
    /// 惰性初始化（首次访问时建一条），线程退出时随 TLS 销毁。销毁时 `LocalSet`
    /// 里未完成的任务被丢弃，这正是「队列寿命 = 线程寿命」的具体含义。
    static LOCAL_QUEUE_: Rc<tokio::task::LocalSet> = Rc::new(tokio::task::LocalSet::new());
}

/// tokio 后端的本地作用域：**本线程那条 `LocalSet` 的别名**。
///
/// # 线程独占
///
/// 它是 `!Send` 的：队列绑定本线程，`run_until` 只能在**这条**线程上驱动它。
/// 需要在别的线程上投递本地任务时，在那边另取一个作用域（`Runtime::local_scope()`）。
///
/// # 克隆与「再取一次」是同一件事
///
/// [`Clone`] 只增加别名（同一个 `Rc<LocalSet>`、同一个运行时把手）。同一线程上再次
/// 调 `Runtime::local_scope()` 得到的也是**同一条**队列——两者都**不会**新建队列。
pub struct LocalScope {
    /// 取得本作用域时抓住的运行时句柄。
    ///
    /// `run_until` 不需要它（驱动 `LocalSet` 只需队列本身）；它服务于
    /// `mock-clock` feature 下的 [`LocalScope::block_on_advancing`] 与 escape hatch
    /// [`LocalScope::handle`]。
    handle_: tokio::runtime::Handle,
    /// 本线程那条本地队列的别名（来自 `thread_local!`）。
    local_: Rc<tokio::task::LocalSet>,
}

impl LocalScope {
    /// 后端起内部构造：由 `Runtime::local_scope()` 交出。
    ///
    /// 刻意**不**公开：作用域只能从运行时值取得，这样它的来源与能力位声明
    /// （`Runtime<CAPS>::local_scope()` 要求 `SPAWN_LOCAL`）都是可追溯的。
    ///
    /// 队列本身来自本线程的 TLS——本函数**不**新建队列。
    pub(crate) fn with_handle(handle: tokio::runtime::Handle) -> Self {
        Self {
            handle_: handle,
            local_: LOCAL_QUEUE_.with(Rc::clone),
        }
    }

    /// 本作用域抓住的 tokio 句柄（escape hatch）。
    pub fn handle(&self) -> &tokio::runtime::Handle {
        &self.handle_
    }
}

impl Clone for LocalScope {
    /// 多加一个别名：同一条队列、同一个运行时把手。
    fn clone(&self) -> Self {
        Self {
            handle_: self.handle_.clone(),
            local_: Rc::clone(&self.local_),
        }
    }
}

impl fmt::Debug for LocalScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("abs_art_tokio::LocalScope")
            .finish_non_exhaustive()
    }
}

impl TrLocalScope for LocalScope {
    /// 与全局 `spawn` 共用同一个 [`JoinHandle`]。
    type Handle<T>
        = JoinHandle<T>
    where
        T: 'static;

    /// 把 `future` 投递到本线程的本地队列。
    ///
    /// 队列随**线程**存活，不随任务句柄存活——因此
    /// [`TrJoinHandle::detach`](abs_art::TrJoinHandle::detach) 之后任务仍会被持续
    /// 驱动，直到它自己结束（或线程退出）。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        self.local_.spawn_local(future).into()
    }

    /// 驱动本线程的本地队列直到 `future` 完成（`LocalSet::run_until`）。
    ///
    /// 需要外层已有一个 tokio 运行时在驱动本 future——典型写法是
    /// `rt.block_on(scope.run_until(fut))`（在运行时上下文之外）或
    /// `scope.run_until(fut).await`（已在上下文内）。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future,
    {
        self.local_.run_until(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端「本地作用域」的单元测试。

    use std::{cell::Cell, rc::Rc, time::Duration};

    use abs_art::{FULL, SPAWN_LOCAL, TrDelay, TrJoinHandle, TrLocalScope};

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

    /// 建一个 current_thread 的 tokio 运行时（契约只需要 time 驱动）。
    fn rt_() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("建 tokio 运行时")
    }

    /// 目的：验证运行时**值**是 `Send + Sync`——本地队列不在它里面。
    ///
    /// 实施策略：编译期断言 `Runtime<FULL>: Send + Sync`。
    ///
    /// 通过依据：编译通过即为通过。这正是把队列从运行时值里剥出去换来的性质：
    /// 拿得到全局 `spawn` 能力的把手可以跨线程传。
    #[test]
    fn runtime_value_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime<{ FULL }>>();
    }

    /// 目的：验证 `run_until` 会驱动本线程的本地队列，`!Send` 任务能跑完。
    ///
    /// 实施策略：在 tokio 运行时里由运行时值交出作用域，投递一个捕获 `Rc<u32>`
    /// 的本地任务，用 `run_until` 驱动并 await 其句柄。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若队列没有被驱动，await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        in_fresh_thread_(|| {
            let rt = rt_();

            let out = rt.block_on(async {
                let scope = crate::current().local_scope();
                scope
                    .run_until(async {
                        let rc = Rc::new(6u32);
                        let handle = scope.spawn_local(async move { *rc * 7 });
                        handle.await.unwrap()
                    })
                    .await
            });

            assert_eq!(out, 42);
        });
    }

    /// 目的：验证本地队列的寿命与线程绑定——`detach()` 之后任务仍继续运行。
    ///
    /// 实施策略：在 `scope.run_until` 内投递一个置位 `Rc<Cell<bool>>` 的本地任务后
    /// 立即 `detach()`，再循环 `yield_now` 等标志置位。
    ///
    /// 通过依据：标志在有限次让出内被置位；若实现把队列绑在句柄上（drop 即取消），
    /// 循环会因超出上限而断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        in_fresh_thread_(|| {
            let rt = rt_();

            rt.block_on(async {
                let scope = crate::current().local_scope();
                scope
                    .run_until(async {
                        let flag = Rc::new(Cell::new(false));
                        let task_flag = Rc::clone(&flag);

                        let handle = scope.spawn_local(async move {
                            tokio::task::yield_now().await;
                            task_flag.set(true);
                        });
                        handle.detach();

                        let mut spins = 0u32;
                        while !flag.get() {
                            tokio::task::yield_now().await;
                            spins += 1;
                            assert!(spins < 1_000_000, "detach 后本地任务未被推进");
                        }
                    })
                    .await;
            });
        });
    }

    /// 目的：验证「声明 → 取得」路径——只写 `SPAWN_LOCAL` 的运行时值确实交得出作用域。
    ///
    /// 实施策略：把 CAPS 写成只含 `SPAWN_LOCAL`，经 `local_scope()` 取作用域并跑一个
    /// 捕获 `Rc` 的 `!Send` 任务。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错（例如标在别的
    /// 位上），本测试将无法编译。
    #[test]
    fn declared_cap_gives_usable_local_queue() {
        in_fresh_thread_(|| {
            let rt = rt_();

            let out = rt.block_on(async {
                let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();
                scope
                    .run_until(async {
                        let rc = Rc::new(6u32);
                        scope.spawn_local(async move { *rc * 7 }).await.unwrap()
                    })
                    .await
            });

            assert_eq!(out, 42);
        });
    }

    /// 目的：验证同一个作用域的克隆**共享同一条**本地队列。
    ///
    /// 实施策略：克隆作用域，用克隆体投递任务，用原作用域驱动，再 await 句柄。
    ///
    /// 通过依据：取回 5——若克隆各有各的队列，驱动原作用域不会推进克隆体投递的
    /// 任务，await 会挂起。
    #[test]
    fn clones_share_one_local_queue() {
        in_fresh_thread_(|| {
            let rt = rt_();

            let out = rt.block_on(async {
                let scope = crate::current().local_scope();
                let clone = scope.clone();
                let handle = clone.spawn_local(async { 5u32 });
                scope.run_until(handle).await.unwrap()
            });

            assert_eq!(out, 5);
        });
    }

    /// 目的：验证**本线程上多次 `local_scope()` 是同一条队列**（幂等的别名语义）。
    ///
    /// 实施策略：取两个作用域 A、B（**不**做 clone），用 A 投递、用 B 驱动。
    ///
    /// 通过依据：取回 5。修复前每个 `local_scope()` 会新建一条 `LocalSet`，此用例
    /// 会永久挂起——它正是本次「队列进 `thread_local!`」改造的回归闸门。
    #[test]
    fn separate_calls_share_the_thread_local_queue() {
        in_fresh_thread_(|| {
            let rt = rt_();

            let out = rt.block_on(async {
                let value = crate::current();
                let a = value.local_scope();
                let b = value.local_scope();
                let handle = a.spawn_local(async { 5u32 });
                b.run_until(handle).await.unwrap()
            });

            assert_eq!(out, 5);
        });
    }

    /// 目的：验证本地投递与 `delay` 各自挂在正确的宿主上（作用域管队列、运行时管计时）。
    ///
    /// 实施策略：在 `scope.run_until` 内用**运行时值**的 `delay` 睡 1ms，量墙上耗时。
    ///
    /// 通过依据：耗时 ≥ 1ms；编译通过本身也证明作用域上没有 `delay`（它在 `rt` 上）。
    #[test]
    fn delay_comes_from_the_runtime_not_the_scope() {
        in_fresh_thread_(|| {
            let rt = rt_();

            let elapsed = rt.block_on(async {
                let value = crate::current();
                let scope = value.local_scope();
                let started = std::time::Instant::now();
                scope
                    .run_until(async {
                        value.delay(Duration::from_millis(1)).await;
                    })
                    .await;
                started.elapsed()
            });

            assert!(elapsed >= Duration::from_millis(1), "耗时为 {elapsed:?}");
        });
    }

    /// 目的：验证 `TrLocalScope::Handle` 与 `crate::JoinHandle` 是同一个类型。
    ///
    /// 实施策略：把 `spawn_local` 交回的句柄直接传给形参类型为 `crate::JoinHandle<u32>`
    /// 的函数。
    ///
    /// 通过依据：编译通过即为通过（类型相等）。
    #[test]
    fn handle_type_is_the_shared_join_handle() {
        in_fresh_thread_(|| {
            fn take_handle_(_: crate::JoinHandle<u32>) {}

            let rt = rt_();
            rt.block_on(async {
                let scope = crate::current().local_scope();
                take_handle_(scope.spawn_local(async { 1u32 }));
            });
        });
    }
}

#[cfg(feature = "mock-clock")]
impl LocalScope {
    /// 用**手动时钟**驱动本作用域：队列照常被驱动，而时间由测试自己推进。
    ///
    /// 这是「同一份业务代码跑虚拟时间测试」的入口：`body` 里用
    /// [`abs_art_mock_clock::ManualTime`] 的 `delay` / `interval` / `now`，驱动会在
    /// 「没有别的活可干」时把时钟推进到下一个到期时刻。
    ///
    /// # 它**不属于** [`TrLocalScope`](abs_art::TrLocalScope)
    ///
    /// 这是本后端在 `mock-clock` feature 下提供的**固有方法**，不是作用域契约的一部分
    /// ——抽象层的作用域只有 [`spawn_local`](abs_art::TrLocalScope::spawn_local) 与
    /// [`run_until`](abs_art::TrLocalScope::run_until) 两条。它是「阻塞 + 驱动」的
    /// 测试专用入口，名字里的 `block_on` 只描述它自己做的那次阻塞。
    ///
    /// # 上下文
    ///
    /// - 在运行时上下文**之外**调用：直接 `Handle::block_on`；
    /// - 在上下文**之内**调用：走 `block_in_place` 让渡当前 worker，因此要求多线程运行时
    ///   （`current_thread` 下该分支会 panic，与 tokio 的规定一致）。**不得**在
    ///   `LocalSet` 的驱动栈内调用（tokio 禁止在 `LocalSet` 内 `block_in_place`）。
    ///
    /// # Panics
    ///
    /// 主体既没有就绪任务、也没有可推进的定时器时 panic（把静默挂起变成响亮失败）。
    pub fn block_on_advancing<F, C>(&self, clock: &C, body: F) -> <F as Future>::Output
    where
        F: Future,
        C: abs_art_mock_clock::ManualClockApi,
    {
        let handle = self.handle_.clone();
        let local = Rc::clone(&self.local_);
        let supervisor = abs_art_mock_clock::Supervisor::new(body, clock.clone(), || false);
        let run = move || handle.block_on(local.run_until(supervisor));
        match tokio::runtime::Handle::try_current() {
            Ok(_) => tokio::task::block_in_place(run),
            Err(_) => run(),
        }
    }
}

#[cfg(all(test, feature = "mock-clock"))]
mod mock_clock_tests_ {
    //! tokio 后端接上手动时钟之后的虚拟时间行为。

    use std::{rc::Rc, time::Duration};

    use abs_art::{TrClock, TrDelay, TrLocalScope};
    use abs_art_mock_clock::{ManualClock, ManualTime, MockInstant};

    /// 在独立线程上运行（理由见 `tests::in_fresh_thread_`）。
    fn in_fresh_thread_<F>(f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::spawn(f).join().expect("用例线程 panic");
    }

    /// 建一个多线程 tokio 运行时（`block_in_place` 分支需要）。
    fn rt_() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("建多线程 tokio 运行时")
    }

    /// 目的：验证「虚拟一小时」在真实时间里几乎瞬间完成，且时刻与计时器同源。
    ///
    /// 手段：取运行时值与作用域，用 `ManualTime` 装饰后 `block_on_advancing` 跑
    /// `delay(1h)`，记录真实耗时与虚拟时刻。
    ///
    /// 通过依据：真实耗时 < 1s（虚拟时间没有被真的等），虚拟时刻恰好是 3600_000ms。
    #[test]
    fn virtual_hour_passes_instantly() {
        in_fresh_thread_(|| {
            let rt = rt_();
            let (scope, value) = rt.block_on(async {
                let value = crate::current();
                let scope = value.local_scope();
                (scope, value)
            });

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
            assert_eq!(clock.now().as_millis(), 3_600_000);
        });
    }

    /// 目的：验证**投递到本地队列的任务**同样跑在虚拟时间上。
    ///
    /// 手段：`spawn_local` 一个睡虚拟 30 分钟的任务（任务内自建 `ManualTime`），
    /// 主体 await 它的句柄，全程由 `block_on_advancing` 驱动。
    ///
    /// 通过依据：取回 42，虚拟时刻推进到 1800_000ms，真实耗时 < 1s。
    #[test]
    fn spawned_local_task_runs_on_virtual_time() {
        in_fresh_thread_(|| {
            let rt = rt_();
            let (scope, value) = rt.block_on(async {
                let value = crate::current();
                let scope = value.local_scope();
                (scope, value)
            });

            let clock = ManualClock::new();
            let started = std::time::Instant::now();
            // 作用域是 `Clone` 的（共享同一条队列），主体里用的是克隆体。
            let scope_in_body = scope.clone();
            let out = scope.block_on_advancing(&clock, {
                let clock = clock.clone();
                let inner = value;
                async move {
                    let handle = scope_in_body.spawn_local({
                        let clock = clock.clone();
                        async move {
                            let timed = ManualTime::new(inner, clock);
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
            let _ = Rc::new(());
        });
    }
}
