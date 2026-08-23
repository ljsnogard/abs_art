use compio::runtime::Runtime as CompioRuntime;

use crate::runtime::{Runtime, TrBlockOn};

impl Runtime {
    /// 阻塞当前线程，等待 `future` 完成，同时不影响 compio 运行时的调度。
    ///
    /// 必须在 compio 运行时上下文内调用（例如在 `Runtime::block_on` 或某个
    /// 由 `Runtime::spawn` 创建的任务内部）；否则 `Runtime::with_current`
    /// 会 panic。
    pub fn block_on<F>(future: F) -> F::Output
    where
        Self: TrBlockOn<F>,
        F: Future + 'static,
    {
        <Runtime as TrBlockOn<F>>::block_on(future)
    }
}

impl<F> TrBlockOn<F> for Runtime
where
    F: Future + 'static,
{
    /// 通过 `CompioRuntime::with_current` 获取当前线程的环境运行时，再调用其
    /// `block_on` 阻塞驱动 `future`。
    ///
    /// compio 的 `block_on` 在等待期间会循环执行「轮询 future → 驱动 executor
    /// → 轮询驱动」：因此 `future` 处于 pending 时，同运行时内 `spawn` 的其他
    /// 任务仍会被推进，即「不影响运行时调度」。
    fn block_on(future: F) -> F::Output {
        CompioRuntime::with_current(|rt| {
            rt.block_on(future)
        })
    }
}
