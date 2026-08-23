use tokio::runtime::Handle;

use crate::runtime::{Runtime, TrBlockOn};

impl Runtime {
    /// 阻塞当前线程，等待 `future` 完成，同时不影响 tokio 运行时的调度。
    ///
    /// 必须在 tokio 运行时上下文内调用（例如在 `Runtime::block_on` 或某个
    /// 由 `tokio::spawn` 创建的任务内部）；否则 `Handle::current()` 会 panic。
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
    /// 先通过 `tokio::task::block_in_place` 把当前 worker 线程（及其任务队列）
    /// 让渡回阻塞线程池，让其他任务可以继续在该线程上被调度；再在闭包内用
    /// `Handle::current().block_on(future)` 驱动 `future` 直到完成。
    ///
    /// 这是 tokio 官方文档推荐的「在多线程运行时内同步等待 async 结果」的模式：
    /// 当前线程被阻塞的同时，运行时的其他任务仍能得到调度，即「不影响运行时调度」。
    ///
    /// 注意：`block_in_place` 不允许在 current_thread 运行时内使用（没有其他
    /// worker 线程可以承接任务），此时会 panic。
    fn block_on(future: F) -> <F as Future>::Output {
        tokio::task::block_in_place(move || {
            // 在闭包内，通过当前运行时句柄的 block_on 来等待
            Handle::current().block_on(future)
        })
    }
}
