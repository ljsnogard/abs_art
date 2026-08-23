use crate::runtime::{Runtime, TrBlockOn};

impl Runtime {
    /// 阻塞当前线程，等待 `future` 完成，同时不影响 smol 全局执行器的调度。
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
