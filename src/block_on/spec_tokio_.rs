use tokio::runtime::Handle;

use crate::runtime::{Runtime, TrBlockOn};

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
