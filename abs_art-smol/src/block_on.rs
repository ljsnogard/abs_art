//! `block_on`：阻塞当前线程等待 future 完成——**只等待，不驱动本地队列**。
//!
//! smol 的 `block_on`（底层是 `async_io::block_on`）不依赖任何「环境运行时」：
//! 它在当前线程上直接轮询 future 并驱动 async-io 的反应器，因此本入口**没有先决
//! 条件**（tokio 的 `block_in_place` 那种「多线程运行时」前提在 smol 上不存在）。
//!
//! # 它不涉及本地队列
//!
//! 本地队列不归运行时值所有（smol 的值是零大小标记，见 `lib.rs`；队列在本线程的
//! `thread_local!` 里），所以本方法**只等待**，不驱动任何 `!Send` 任务的队列。
//! 需要「等待期间继续驱动本线程的本地队列」时，把
//! [`TrLocalScope::run_until`](abs_art::TrLocalScope::run_until) 交给一个正在跑的
//! 驱动源去 await——例如 `smol::block_on(scope.run_until(f))`。抽象层的作用域上
//! **没有**阻塞入口。

use core::future::Future;

use abs_art::{HasBlockOn, TrBlockOn};

use crate::Runtime;

impl<const CAPS: usize> TrBlockOn for Runtime<CAPS>
where
    [(); CAPS]: HasBlockOn,
{
    /// 阻塞当前线程直到 `future` 完成。
    ///
    /// smol 没有「环境运行时句柄」，因此这里不需要任何让渡动作：直接
    /// `smol::block_on(future)`。全局执行器由 smol 的后台线程驱动；任何本地队列
    /// 都不归本值所有，因此**不被本方法驱动**。
    ///
    /// # Panics
    ///
    /// 不 panic。
    fn block_on<F>(&self, future: F) -> <F as Future>::Output
    where
        F: Future,
    {
        smol::block_on(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `Runtime::block_on` 单元测试。
    //!
    //! smol 的 `block_on` 不依赖任何「环境运行时句柄」，因此这些测试与
    //! tokio/compio 的测试在**前提**上有所不同：这里全部是「无前提」的用例。

    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use abs_art::{BLOCK_ON, SPAWN_SEND, TrBlockOn};

    use crate::Runtime;

    /// 目的：验证 smol 后端的 `block_on` **不依赖任何环境运行时**——与 tokio
    /// （要求 `Handle::current()`）和 compio（要求 `Runtime::with_current`）不同，
    /// 它可以在从未创建过任何运行时的线程里直接使用。
    ///
    /// 手段：不创建、不进入任何运行时上下文，直接构造运行时值并调用它的
    /// `block_on` 驱动一个返回常量表达式的 future。
    ///
    /// 判定：返回值为 `6 * 7 == 42`，且整个过程没有 panic。
    #[test]
    fn block_on_without_runtime_context_returns_output() {
        let value = crate::current();
        let out = value.block_on(async { 6 * 7 });

        assert_eq!(out, 42);
    }

    /// 目的：验证 `block_on` 可以嵌套在另一个 `block_on` 上下文内部使用
    /// （async-io 的 `block_on` 为递归调用单独创建 parker，两层互不干扰）。
    ///
    /// 手段：先在外层 `smol::block_on` 内，再在内层调用运行时值的 `block_on`
    /// 驱动另一个 future，把内层结果带出外层。
    ///
    /// 判定：内层与外层都得到 `42`，且没有 panic。
    #[test]
    fn block_on_nested_inside_block_on() {
        let value = crate::current();
        let out = smol::block_on(async {
            let inner = value.block_on(async { 6 * 7 });
            assert_eq!(inner, 42);
            inner
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证 `block_on` 阻塞当前线程期间，smol 的**全局执行器**（由 smol
    /// 的后台线程驱动）仍能推进——即「不影响异步运行时的调度」这一契约。
    ///
    /// 手段：先用 `smol::spawn` 向全局执行器提交一个后台任务，它在循环中递增原子
    /// 计数器并 `yield_now`；再调用值的 `block_on` 阻塞等待计数器达到目标值
    /// （等待循环用 `smol::future::yield_now` 让出，使 async-io 的 `block_on`
    /// 循环继续轮询）。整个场景放进独立线程并用 `recv_timeout` 限时。
    ///
    /// 判定：若 10 秒内 `block_on` 返回且计数达到目标值（10_000），说明后台任务在
    /// 阻塞期间确实被调度；否则超时失败（由 `recv_timeout` 的 panic 体现）。
    #[test]
    fn block_on_does_not_block_global_executor() {
        const TARGET: usize = 10_000;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let counter = std::sync::Arc::new(AtomicUsize::new(0));
            let c = counter.clone();

            let task = smol::spawn(async move {
                for _ in 0..TARGET {
                    c.fetch_add(1, Ordering::Relaxed);
                    smol::future::yield_now().await;
                }
                c.load(Ordering::Relaxed)
            });

            let wait_counter = counter.clone();
            let value = crate::current();
            let result = value.block_on(async move {
                while wait_counter.load(Ordering::Relaxed) < TARGET {
                    smol::future::yield_now().await;
                }
                task.await
            });

            let _ = tx.send(result);
        });

        let result = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("测试失败：等待结果超时或线程 panic（{e:?}）"));
        assert_eq!(result, TARGET);
    }

    /// 目的：验证能力位模式下，声明了 `BLOCK_ON` 的 `Runtime<CAPS>` **值**确实
    /// 实现了 `TrBlockOn`（编译期能力检查的正向用例）。
    ///
    /// 手段：用 `Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current()` 构造值，再经
    /// trait 方法 `block_on` 驱动一个 future。
    ///
    /// 判定：返回值为 `40 + 2 == 42`；若 `HasBlockOn` 标记或条件化 trait impl 有误，
    /// 本测试无法编译。
    #[test]
    fn tagged_runtime_implements_block_on() {
        let value = Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current();
        let out = value.block_on(async { 40 + 2 });
        assert_eq!(out, 42);
    }

    /// 目的：验证 `Runtime::block_on` **不驱动**本地作用域的队列——「只等待」与
    /// 「驱动本地队列」两个入口分工明确（后者在 `LocalScope` 的 `run_until` 上）。
    ///
    /// 手段：取运行时值与它的作用域（在**独立线程**上，保证拿到干净的线程本地队列）；
    /// 在作用域上投递一个置位 `Rc<Cell<bool>>` 的本地任务并 `detach()`（避免句柄 drop
    /// 取消任务）；先只调 `value.block_on(..)` 让出若干次，断言标志仍为 false；最后用
    /// `smol::block_on(scope.run_until(..))` 驱动队列，等标志置位。
    ///
    /// 判定：`value.block_on` 之后标志为 false（若它偷偷驱动了队列，这里就变 true
    /// 而断言失败）；`run_until` 之后标志为 true（若作用域没驱动队列，等待循环会因
    /// 超出上限而断言失败）。
    #[cfg(feature = "local_scope")]
    #[test]
    fn block_on_does_not_drive_the_local_scope_queue() {
        std::thread::spawn(|| {
            use std::{cell::Cell, rc::Rc};

            use abs_art::{TrJoinHandle, TrLocalScope};

            let value = crate::current();
            let scope = value.local_scope();

            let flag = Rc::new(Cell::new(false));
            let task_flag = flag.clone();
            let handle = scope.spawn_local(async move {
                smol::future::yield_now().await;
                task_flag.set(true);
            });
            handle.detach();

            // 只让运行时值等待：它不驱动任何本地队列。
            value.block_on(async {
                for _ in 0..16 {
                    smol::future::yield_now().await;
                }
            });
            assert!(!flag.get(), "`Runtime::block_on` 不该驱动本地队列");

            // 由作用域的 `run_until` 驱动才推进。
            let mut spins = 0u32;
            smol::block_on(scope.run_until(async {
                while !flag.get() {
                    smol::future::yield_now().await;
                    spins += 1;
                    assert!(spins < 1_000_000, "作用域未能驱动本地任务");
                }
            }));
            assert!(flag.get());
        })
        .join()
        .expect("用例线程 panic");
    }
}
