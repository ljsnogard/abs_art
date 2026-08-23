//! `block_on`：阻塞当前线程等待 future 完成。

use core::future::Future;

use crate::Runtime;
use abs_art::{FULL, HasBlockOn, TrBlockOn};

impl Runtime<FULL> {
    /// 阻塞当前线程，等待 `future` 完成。
    ///
    /// smol 的 `block_on` 不依赖任何「环境运行时」，可以在任何线程直接使用。
    pub fn block_on<F>(future: F) -> F::Output
    where
        Self: TrBlockOn<F>,
        F: Future + 'static,
    {
        <Self as TrBlockOn<F>>::block_on(future)
    }
}

impl<F, const CAPS: usize> TrBlockOn<F> for Runtime<CAPS>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
    [(); CAPS]: HasBlockOn,
{
    /// 直接调用 `smol::block_on`（其底层是 `async_io::block_on`）：在当前
    /// 线程上轮询 future 并驱动 async-io reactor，直到 future 完成。
    ///
    /// smol 没有「环境运行时句柄」的概念：`smol::spawn` 提交的任务由独立的
    /// 后台线程（全局执行器）驱动，因此本函数既不需要预先进入任何运行时上下文，
    /// 也不会阻塞全局执行器的调度。
    fn block_on(future: F) -> F::Output {
        smol::block_on(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `Runtime::block_on` 单元测试。
    //!
    //! smol 的 `block_on`（底层为 `async_io::block_on`）会在当前线程上直接
    //! 轮询 future 并驱动 async-io reactor，不依赖任何「环境运行时句柄」，
    //! 因此这些测试与 tokio/compio 的测试在前提上有所不同。

    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use crate::Runtime;

    /// 目的：验证 smol 后端的 `Runtime::block_on` 不依赖任何「环境运行时」——
    /// 与 tokio（需要 `Handle::current()`）和 compio（需要 `Runtime::with_current`）
    /// 不同，它可以在任何线程中直接使用，包括从未创建过 smol 运行时的线程。
    ///
    /// 实施策略：不创建任何运行时、不进入任何上下文，直接调用 `Runtime::block_on`
    /// 驱动一个返回常量表达式的 future。
    ///
    /// 通过依据：返回值为 6 * 7 == 42，且整个过程没有 panic。
    #[test]
    fn block_on_without_runtime_context_returns_output() {
        let out = Runtime::block_on(async { 6 * 7 });

        assert_eq!(out, 42);
    }

    /// 目的：验证 `Runtime::block_on` 可以嵌套在另一个 `block_on` 上下文内部使用
    /// （递归调用不会互相干扰，async-io 的 `block_on` 为递归调用单独创建 parker）。
    ///
    /// 实施策略：先在外层 `smol::block_on` 中进入执行器上下文，再在内层调用
    /// `Runtime::block_on` 驱动另一个 future，把内层结果带出外层。
    ///
    /// 通过依据：内外两层都返回预期值（内层 == 外层 == 42），且没有 panic。
    #[test]
    fn block_on_nested_inside_block_on() {
        let out = smol::block_on(async {
            let inner = Runtime::block_on(async { 6 * 7 });
            assert_eq!(inner, 42);
            inner
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证 `Runtime::block_on` 阻塞当前线程期间，smol 全局执行器
    /// （`smol::spawn` 提交的任务，运行在独立的后台线程上）仍能正常推进——
    /// 即「不影响异步运行时的调度」这一契约。
    ///
    /// 实施策略：先用 `smol::spawn` 向全局执行器提交一个后台任务，它在循环中
    /// 递增原子计数器并 `yield_now`；然后调用 `Runtime::block_on` 让当前线程
    /// 阻塞等待计数器达到目标值（等待循环使用 `smol::future::yield_now` 让出，
    /// 使 async-io 的 `block_on` 循环能够继续轮询）。整个场景放进独立线程并用
    /// `recv_timeout` 限时。
    ///
    /// 通过依据：若 10 秒内 `Runtime::block_on` 返回且计数达到目标值（10_000），
    /// 说明后台任务在 block_on 阻塞期间确实被调度执行了，测试通过；若全局执行器
    /// 被干扰而无法推进，等待循环将永远无法退出，超时失败。
    /// 目的：验证 Tag 模式下，声明了 `BLOCK_ON` 能力的 `Runtime<Caps>` 确实
    /// 实现了 `TrBlockOn`（编译期能力检查的正向用例）。
    ///
    /// 实施策略：用 `Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>` 的 trait 关联函数
    /// 调用 `block_on` 驱动一个 future。
    ///
    /// 通过依据：返回值为 40 + 2 == 42；若 `HasBlockOn` 标记或条件化 trait
    /// impl 有误，将无法编译。
    #[test]
    fn tagged_runtime_implements_block_on() {
        use abs_art::{BLOCK_ON, SPAWN_LOCAL, TrBlockOn};

        let out = <Runtime<{ BLOCK_ON | SPAWN_LOCAL }> as TrBlockOn<_>>::block_on(
            async { 40 + 2 },
        );
        assert_eq!(out, 42);
    }

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
            let result = Runtime::block_on(async move {
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
}
