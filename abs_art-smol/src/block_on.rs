//! `block_on`：阻塞当前线程等待 future 完成，同时驱动本值的本地队列。
//!
//! smol 的 `block_on`（底层是 `async_io::block_on`）不依赖任何「环境运行时」：
//! 它在当前线程上直接轮询 future 并驱动 async-io 的反应器，因此本入口**没有先决
//! 条件**（tokio 的 `block_in_place` 那种「多线程运行时」前提在 smol 上不存在）。
//!
//! 值化之后，`block_on` 同时是**本地队列的阻塞驱动入口**：本值持有的
//! `LocalExecutor` 与传入的 future 被同一个 `smol::block_on` 一起驱动。

use core::future::Future;

use abs_art::{HasBlockOn, TrBlockOn};

use crate::Runtime;

impl<const CAPS: usize> TrBlockOn for Runtime<CAPS>
where
    [(); CAPS]: HasBlockOn,
{
    /// 阻塞当前线程直到 `future` 完成，期间持续驱动**本值**持有的本地队列。
    ///
    /// smol 没有「环境运行时句柄」，因此这里不需要任何让渡动作，也不会堵塞
    /// 全局执行器：全局任务由 smol 的后台线程驱动，与本调用无关。
    ///
    /// # Panics
    ///
    /// 不 panic。
    fn block_on<F>(&self, future: F) -> <F as Future>::Output
    where
        F: Future,
    {
        #[cfg(feature = "local_scope")]
        {
            smol::block_on(self.local_.run(future))
        }
        #[cfg(not(feature = "local_scope"))]
        {
            smol::block_on(future)
        }
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

    /// 目的：验证 `block_on` 会驱动**本值**持有的本地队列——值化之后「阻塞等待」
    /// 与「驱动本地队列」是同一个动作，调用方不必再单独拿一个作用域值去驱动。
    ///
    /// 手段：构造运行时值，用它的 `block_on` 驱动一个 `!Send`（捕获 `Rc`）的本地
    /// 任务并 await 其句柄。
    ///
    /// 判定：取回 `6 * 7 == 42`；若 `block_on` 没有驱动本地队列，await 会永久挂起。
    #[cfg(feature = "local_scope")]
    #[test]
    fn block_on_drives_the_values_local_queue() {
        use abs_art::TrLocalScope;

        let value = crate::current();
        let out = value.block_on(async {
            let rc = std::rc::Rc::new(6u32);
            value.spawn_local(async move { *rc * 7 }).await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
