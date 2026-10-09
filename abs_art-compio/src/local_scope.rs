//! `local_scope`：本地队列的**持有者与驱动点**——一个线程独占的作用域值。
//!
//! # 为什么本后端的队列是「运行时自带的」
//!
//! compio 的运行时**本身就是线程本地的**（`compio::runtime::Runtime` 内部是
//! `Rc<Executor>` + `Rc<RefCell<Proactor>>`，`!Send`，不能跨线程发送），并且：
//!
//! - `Runtime::spawn` **不要求** `F: Send`，它投的就是这份运行时自己的执行器队列；
//! - 这条队列归运行时所有，由运行时自己在 `block_on` / `wait` 期间 tick 驱动；
//! - 因此 compio 上**不存在** tokio 那种「与运行时把手可分离的 `LocalSet`」——
//!   队列不可分离，它就是运行时实例的一部分。
//!
//! 与抽象层其他后端（tokio 的 `LocalSet`、smol 的 `LocalExecutor`）对比，这条差异是
//! 各后端「如实表达自己前提」的一部分：[`LocalScope`] 在本后端**不是**另建一条队列，
//! 而是**钉住那份已经带着队列的运行时**。
//!
//! # 它为什么仍然要是一个独立的值
//!
//! 队列虽然不可分离，但「**线程独占**」这条性质必须被表达出来：[`TrLocalScope`] 描述的是
//! 「哪条线程上投递、由谁驱动」，而 [`crate::Runtime`] 描述的是「哪份运行时、哪种计时」。
//! 两者在 compio 上恰好指向同一个实例，却是两种不同的关切：
//!
//! - 抽象层的 [`TrLocalScope`] 不提供计时（[`TrDelay`](abs_art::TrDelay) /
//!   [`TrClock`](abs_art::TrClock) / [`TrTime`](abs_art::TrTime) 只挂在运行时值上），
//!   于是「作用域管队列、运行时管时间」这条分工在三个后端的形状一致；
//! - 作用域是 `!Send` 的（它内部持有 `compio::runtime::Runtime`，后者含 `Rc`），
//!   这正是「线程独占」的忠实表达：拿到作用域就不能把它搬去别的线程，想在那条线程上
//!   投递就得在那边另取一个。
//!
//! # 取得路径只有一条
//!
//! `Runtime<CAPS>::local_scope()`（要求 `CAPS` 含
//! [`SPAWN_LOCAL`](abs_art::SPAWN_LOCAL)）。作用域不会脱离运行时值凭空出现：这也保证了
//! 「这次 `spawn_local` 投到哪份运行时」由值回答，而不是靠当前线程恰好进入了哪个运行时。
//!
//! # 保留的语义：[`TrLocalScope::run_until`] 等价于直接 await
//!
//! compio 的队列由运行时自己驱动，**不存在**「需要调用方额外驱动的队列」这回事
//! （对照 tokio 的 `LocalSet::run_until`）。因此本后端的 `run_until(future)`
//! **原样返回 `future`**：驱动来自外层已经在跑的那个 compio `block_on` / `wait`。
//!
//! # 阻塞入口：[`TrLocalScope::block_on_local`] 自己 tick 执行器与 IO
//!
//! 正因为队列不可分离，compio 上的 [`block_on_local`](TrLocalScope::block_on_local)
//! 不需要「park 出去等别人驱动」：它握着这份运行时，直接跑 compio 自己
//! `Runtime::block_on` 内部那套循环——`轮询 future → tick executor → 有活只探一次 IO /
//! 没活则阻塞等 IO`。因此它**不进入运行时上下文也能用**，且阻塞期间执行器与 IO proactor
//! 都在同一线程上照常推进（这一点与 tokio / smol 相反，那两端的 park 会让本线程的 IO
//! 停摆）。
//!
//! 运行时值的 [`TrBlockOn::block_on`](abs_art::TrBlockOn::block_on) 在 compio 上做的是
//! 同一件事（`self.rt_.block_on` 也 `enter` 并 tick 执行器）；两者的差别只在「谁提供
//! 上下文」：`block_on` 由运行时值提供，`block_on_local` 用作用域已经钉住的那一份。

use core::{
    fmt,
    future::Future,
    task::{Context, Poll},
    time::Duration,
};

use abs_art::TrLocalScope;

use crate::JoinHandle;

/// compio 后端的本地作用域：钉住一份 compio 运行时作为队列与驱动点。
///
/// # 本后端是「作用域形状」的对齐基准
///
/// 家族现在要求三后端的 [`TrLocalScope`] 都是**线程本地那条队列的别名**：`Clone`
/// 只增加别名、同一线程上多次取得拿到同一条队列、类型 `!Send`。compio 天然满足这套
/// 语义——队列不可与运行时实例分离，而 compio 的运行时本身线程绑定，所以「作用域」
/// 就是那份运行时的别名，`local_scope()` 对同一个值幂等。tokio / smol 侧是靠把队列
/// 放进 `thread_local!` 对齐到这条语义的。
///
/// 一处**如实的差异**：compio 允许同一线程上存在多份运行时实例（`Runtime::with_runtime`），
/// 那时每份实例各有自己的队列——这是 compio 运行时自身的性质，不属于本家族的契约面；
/// 正常路径（`Runtime::current()`）下三者可观察语义一致。
///
/// # 线程独占
///
/// 它是 `!Send` 的：内部持有的 `compio::runtime::Runtime` 由 `Rc` 构成，绑定创建它的
/// 线程。需要在别的线程上投递本地任务时，在那边用 `Runtime::local_scope()` 另取一个。
/// 这不是本 crate 附加的限制，而是 compio 运行时本来的性质。
///
/// # 克隆
///
/// [`Clone`] 共享**同一份**运行时（`Rc` 句柄簇）——克隆出来的两个作用域是同一条队列的
/// 两个把手，不是两条队列。
pub struct LocalScope {
    /// 钉住的那份 compio 运行时：`spawn_local` / `block_on` 都打在它上面。
    rt_: compio::runtime::Runtime,
}

impl LocalScope {
    /// 后端起内部构造：由 `Runtime::local_scope()` 交出。
    ///
    /// 刻意**不**公开：作用域只能从运行时值取得，这样它的来源与能力位声明
    /// （`Runtime<CAPS>::local_scope()` 要求 `SPAWN_LOCAL`）都是可追溯的。
    pub(crate) fn with_runtime(rt: compio::runtime::Runtime) -> Self {
        Self { rt_: rt }
    }

    /// 本作用域钉住的那份 compio 运行时（escape hatch，便于做后端特有的事）。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::{SPAWN_LOCAL, TrLocalScope};
    /// use abs_art_compio::Runtime;
    ///
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// rt.block_on(async {
    ///     let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();
    ///     // escape hatch：拿回底层 compio 运行时，问它的驱动类型
    ///     let _driver_type = scope.runtime().driver_type();
    /// });
    /// ```
    pub fn runtime(&self) -> &compio::runtime::Runtime {
        &self.rt_
    }
}

impl Clone for LocalScope {
    /// 共享同一份运行时（同一条队列、同一个驱动）。
    fn clone(&self) -> Self {
        Self {
            rt_: self.rt_.clone(),
        }
    }
}

impl fmt::Debug for LocalScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("abs_art_compio::LocalScope")
            .finish_non_exhaustive()
    }
}

impl TrLocalScope for LocalScope {
    /// 与 `spawn_blocking` 共用同一个 [`JoinHandle`]。
    type Handle<T>
        = JoinHandle<T>
    where
        T: 'static;

    /// 把 `future` 投递到**本作用域钉住的那份运行时**的执行器队列。
    ///
    /// compio 的 `Runtime::spawn` 不要求 `Send`，因此这里可以承载捕获 `Rc` 之类
    /// `!Send` 数据的 future。
    ///
    /// 队列随作用域（以及它所钉住的运行时）存活，**不随任务句柄存活**——因此
    /// [`TrJoinHandle::detach`](abs_art::TrJoinHandle::detach) 之后任务仍会被运行时
    /// 持续驱动，直到它自己结束。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        self.rt_.spawn(future).into()
    }

    /// 驱动**本作用域**的队列，直到 `future` 完成。
    ///
    /// compio 的队列由运行时自己驱动，所以「驱动队列直到 `future` 完成」就是
    /// `future` 本身——本方法**原样返回 `future`**。返回的 future 仍需放在一个正在跑的
    /// compio 上下文里 await（典型：`rt.block_on(scope.run_until(fut))`）。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future,
    {
        future
    }

    /// 阻塞本线程直到 `future` 完成；等待期间自己 tick 本作用域的执行器与 IO proactor。
    ///
    /// 内部就是 compio `Runtime::block_on` 的那套循环（`轮询 future → tick executor →
    /// 有活只探一次 IO / 没活则阻塞等 IO`），因此：
    ///
    /// - 调用点**不必**额外进入 compio 上下文——队列就在本作用域钉住的运行时里；
    /// - 阻塞期间同一线程上既推进队列又推进 IO，不像 tokio / smol 那样让本线程的 IO 停摆。
    ///
    /// # Panics
    ///
    /// 本方法自身不 panic。若 `future` 内部依赖 compio 的**环境式**入口（例如
    /// `TrDelay::delay` 在线程本地注册计时器），注册点看到的仍是当前线程的上下文，
    /// 与调用点是否 `enter` 过一致。
    fn block_on_local<F>(&self, future: F) -> <F as Future>::Output
    where
        F: Future,
    {
        let runtime = &self.rt_;
        let waker = runtime.waker();
        let mut context = Context::from_waker(&waker);
        let mut future = core::pin::pin!(future);
        loop {
            if let Poll::Ready(output) = Future::poll(future.as_mut(), &mut context) {
                // 与 compio 自己的 `block_on` 一致：返回前把队列里剩下的活跑掉。
                runtime.run();
                return output;
            }
            if runtime.run() {
                // 还有排队中的任务：只探一次 IO，不阻塞。
                runtime.poll_with(Some(Duration::ZERO));
            } else {
                // 没有可跑的任务：阻塞等 IO 或下一个定时器（唤醒经本运行时的 waker 到达）。
                runtime.poll();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端「本地作用域」的单元测试。

    use std::{cell::Cell, rc::Rc, time::Duration};

    use abs_art::{SPAWN_LOCAL, TrDelay, TrJoinHandle, TrLocalScope};
    use compio::runtime::Runtime as CompioRuntime;

    // 注意：这里用的是**本 crate 的** `FULL`（不含 `SPAWN_SEND`，即 `59`），
    // 不是 `abs_art::FULL`（`63`，含 `SPAWN_SEND`）。后者会撞上 `CompioCaps_` 断言。
    use crate::{FULL, Runtime};

    /// 目的：验证 `run_until` / 本地投递能让 `!Send` 任务跑完，且结果能经句柄取回。
    ///
    /// 实施策略：在 compio 运行时里由运行时值交出作用域，投递一个捕获 `Rc<u32>` 的本地
    /// 任务，用 `run_until` 驱动并 await 其句柄。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `run_until` 没有把 future 交给正在跑的 compio
    /// 上下文（或本地队列没被驱动），await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let scope = crate::current().local_scope();
            scope
                .run_until(async {
                    let rc = Rc::new(6u32);
                    scope.spawn_local(async move { *rc * 7 }).await.unwrap()
                })
                .await
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证**上下文之外**也能驱动本地队列——compio 的值自带上下文，不需要外层
    /// 已经进入运行时。
    ///
    /// 实施策略：在 compio 上下文之外（测试线程）用 `with_runtime` 造运行时值、交出
    /// 作用域，再用 `value.block_on(scope.run_until(..))` 驱动一个「先 `spawn_local`
    /// 再 await 句柄」的 async 块。捕获 `Rc` 保证任务是 `!Send` 的，只有本地路径能承载。
    ///
    /// 通过依据：返回 `40 + 2 == 42`。这是本后端与 tokio 的一处真实差异：tokio 的
    /// `block_in_place` 需要运行时上下文（`LocalSet` 内甚至直接 panic），而 compio 的
    /// `block_on` 自己 `enter` 并在循环里 tick 执行器队列。
    #[test]
    fn block_on_outside_context_drives_local_queue() {
        use abs_art::TrBlockOn;

        let rt = CompioRuntime::new().unwrap();
        let value = Runtime::<{ FULL }>::with_runtime(rt.clone());
        let scope = value.local_scope();

        let out = value.block_on(scope.run_until(async {
            let rc = Rc::new(40u32);
            scope.spawn_local(async move { *rc + 2 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归作用域钉住的运行时所有——`detach()` 消费句柄后任务继续运行。
    ///
    /// 实施策略：投递一个置位 `Rc<Cell<bool>>` 的本地任务后立即 `detach()`，再用同一个
    /// **运行时值**的 `delay` 反复让出，等标志置位（总共最多 1 秒）。
    ///
    /// 通过依据：标志在期限内被置位；若 detach 实际取消了任务（compio 的 `JoinHandle`
    /// 在 drop 时会 cancel，必须走原生 `detach`），断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        let rt = CompioRuntime::new().unwrap();

        rt.block_on(async {
            let value = crate::current();
            let scope = value.local_scope();
            scope
                .run_until(async {
                    let flag = Rc::new(Cell::new(false));
                    let task_flag = flag.clone();

                    let handle = scope.spawn_local(async move {
                        task_flag.set(true);
                    });
                    handle.detach();

                    let mut elapsed = 0u32;
                    while !flag.get() && elapsed < 1_000 {
                        value.delay(Duration::from_millis(1)).await;
                        elapsed += 1;
                    }
                    assert!(flag.get(), "detach 后本地任务未被推进");
                })
                .await;
        });
    }

    /// 目的：验证「声明 → 取得」路径——只写 `SPAWN_LOCAL` 的运行时值确实交得出作用域。
    ///
    /// 实施策略：把 CAPS 写成只含 `SPAWN_LOCAL`，经 `local_scope()` 取作用域并跑一个捕获
    /// `Rc` 的 `!Send` 任务。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错（例如标在别的位上），
    /// 本测试将无法编译。
    #[test]
    fn declared_cap_gives_usable_local_queue() {
        let rt = CompioRuntime::new().unwrap();

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
    }

    /// 目的：验证同一个作用域的克隆共享**同一份**运行时（`Rc` 句柄簇）——同一条队列。
    ///
    /// 实施策略：克隆作用域，用克隆体投递任务，用原作用域 `run_until` 驱动，再 await 句柄。
    ///
    /// 通过依据：取回 5——若两个克隆指向两份运行时，原作用域驱动的上下文不会推进克隆体
    /// 投递的任务，await 会挂起。
    #[test]
    fn clones_share_one_local_queue() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let scope = crate::current().local_scope();
            let clone = scope.clone();
            let handle = clone.spawn_local(async { 5u32 });
            scope.run_until(handle).await.unwrap()
        });

        assert_eq!(out, 5);
    }

    /// 目的：验证本地投递与 `delay` 各自挂在正确的宿主上（作用域管队列、运行时管计时）。
    ///
    /// 实施策略：在 `scope.run_until` 内用**运行时值**的 `delay` 睡 1ms，量墙上耗时。
    ///
    /// 通过依据：耗时 ≥ 1ms；编译通过本身也证明作用域上没有 `delay`（它在 `rt` 上）。
    #[test]
    fn delay_comes_from_the_runtime_not_the_scope() {
        let rt = CompioRuntime::new().unwrap();

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
    }

    /// 目的：验证 `TrLocalScope::Handle` 与 `crate::JoinHandle` 是同一个类型。
    ///
    /// 实施策略：把 `spawn_local` 交回的句柄直接传给一个形参类型为
    /// `crate::JoinHandle<u32>` 的函数。
    ///
    /// 通过依据：编译通过即为通过（类型相等）。
    #[test]
    fn handle_type_is_the_shared_join_handle() {
        fn take_handle_(_: crate::JoinHandle<u32>) {}

        let rt = CompioRuntime::new().unwrap();
        rt.block_on(async {
            let scope = crate::current().local_scope();
            take_handle_(scope.spawn_local(async { 1u32 }));
        });
    }

    /// 让出一次执行权：自唤醒一次后返回 `Pending`（不依赖 compio 的计时 feature）。
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

    /// 目的：验证 `block_on_local` 在 compio 上自己 tick 执行器，能在**本地队列的驱动栈
    /// 之内**同步等到队列上的任务。
    ///
    /// - 手段：在 `scope.run_until(..)` 内投递一个「让出 5 次后置位标志」的本地任务，再在
    ///   同一调用栈内用 `block_on_local` 同步等标志置位。
    /// - 判定：标志被置位。compio 的 `run_until` 是恒等函数，若 `block_on_local` 只 park
    ///   而不自己 `run` / `poll`，本用例会永久挂起。
    #[test]
    fn block_on_local_waits_inside_the_driving_stack() {
        let rt = CompioRuntime::new().unwrap();
        rt.block_on(async {
            let scope = crate::current().local_scope();
            scope
                .run_until(async {
                    let flag = Rc::new(Cell::new(false));
                    let setter = Rc::clone(&flag);
                    scope
                        .spawn_local(async move {
                            for _ in 0..5 {
                                yield_once_().await;
                            }
                            setter.set(true);
                        })
                        .detach();
                    let waiting = Rc::clone(&flag);
                    scope.block_on_local(async move {
                        while !waiting.get() {
                            yield_once_().await;
                        }
                    });
                    assert!(flag.get(), "供料任务没有被推进");
                })
                .await;
        });
    }

    /// 目的：验证 compio 的**运行时值与作用域都能在创建它们的线程上正常构造并使用**——本
    /// 后端**不**照搬 tokio 那条「运行时值 `Send + Sync`」的性质。
    ///
    /// 实施策略：同一线程内用 `with_runtime` 造运行时值，交出作用域，依次经
    /// `TrBlockOn::block_on` 驱动一个普通 future、以及经
    /// `value.block_on(scope.run_until(..))` 驱动一个本地队列任务。
    ///
    /// 通过依据：两次都取回预期值（编译通过即同时证明两个值都可用于本线程）；compio 的
    /// 运行时含 `Rc`、是 `!Send` 的，因此本用例**刻意不**断言 `Send` / `Sync`——那对本
    /// 后端是假的。
    #[test]
    fn runtime_value_and_scope_are_usable_on_the_creating_thread() {
        use abs_art::TrBlockOn;

        let rt = CompioRuntime::new().unwrap();
        let value = Runtime::<{ FULL }>::with_runtime(rt.clone());
        let scope = value.local_scope();

        assert_eq!(value.block_on(async { 6u32 }), 6);
        assert_eq!(value.block_on(scope.run_until(async { 7u32 })), 7);
    }
}

#[cfg(feature = "mock-clock")]
impl LocalScope {
    /// 用**手动时钟**驱动本作用域：队列照常被驱动，而时间由测试自己推进。
    ///
    /// compio 的执行器就在本作用域钉住的运行时里，因此 tick 钩子直接用它自己的
    /// [`compio::runtime::Runtime::run`]（返回「队列里是否还有任务」）——这正是
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
        let rt = self.rt_.clone();
        let ticker = self.rt_.clone();
        // 「有活」= 上一轮之后有任务被唤醒（唤醒登记）**或** 队列里还有就绪任务。
        // 只取后者会漏掉「跑完最后一个任务、它唤醒的下一环才入队」这一瞬间。
        rt.block_on(abs_art_mock_clock::Supervisor::new(
            body,
            clock.clone(),
            move || ticker.run(),
        ))
    }
}

#[cfg(all(test, feature = "mock-clock"))]
mod mock_clock_tests_ {
    //! compio 后端接上手动时钟之后的虚拟时间行为。

    use std::time::Duration;

    use abs_art::{TrClock, TrDelay, TrJoinHandle, TrLocalScope};
    use abs_art_mock_clock::{ManualClock, ManualTime, MockInstant};

    /// 目的：验证「虚拟一小时」在真实时间里几乎瞬间完成。
    ///
    /// 手段：取运行时值与作用域，`ManualTime` 装饰后 `block_on_advancing` 跑 `delay(1h)`。
    ///
    /// 通过依据：真实耗时 < 1s，虚拟时刻恰为 3600_000ms。
    #[test]
    fn virtual_hour_passes_instantly() {
        let rt = compio::runtime::Runtime::new().expect("建 compio 运行时");
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
    }

    /// 目的：验证**投递到本地队列的任务**同样跑在虚拟时间上。
    ///
    /// 手段：`spawn_local` 一个睡虚拟 30 分钟的任务，主体 await 其句柄。
    ///
    /// 通过依据：取回 42，虚拟时刻推进到 1800_000ms，真实耗时 < 1s。
    #[test]
    fn spawned_local_task_runs_on_virtual_time() {
        let rt = compio::runtime::Runtime::new().expect("建 compio 运行时");
        let (scope, value) = rt.block_on(async {
            let value = crate::current();
            let scope = value.local_scope();
            (scope, value)
        });

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
        let rt = compio::runtime::Runtime::new().expect("建 compio 运行时");
        let (scope, value) = rt.block_on(async {
            let value = crate::current();
            let scope = value.local_scope();
            (scope, value)
        });
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
    }
}
