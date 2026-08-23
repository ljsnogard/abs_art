//! `spawn_send`：把任务投递到 compio 运行时的工作队列。
//!
//! 注意：compio 的运行时是**线程本地**的（不能跨线程发送），因此这里的
//! `spawn` 与 `spawn_local` 行为一致——都是投递到当前线程的环境运行时。

use core::future::Future;

use compio::runtime::Runtime as CompioRuntime;

use abs_art::{FULL, HasSpawnSend, TrSpawnSend};

use crate::{join_handle::JoinHandle, Runtime};

impl Runtime<FULL> {
    /// 把 `future` 投递到当前 compio 运行时的工作队列，返回 [`JoinHandle`]。
    pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
    where
        Self: TrSpawnSend<F>,
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        CompioRuntime::with_current(|rt| rt.spawn(future)).into()
    }
}

impl<F, const CAPS: usize> TrSpawnSend<F> for Runtime<CAPS>
where
    F: Future + Send + 'static,
    <F as Future>::Output: Send + 'static,
    [(); CAPS]: HasSpawnSend,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn spawn(future: F) -> JoinHandle<F::Output> {
        CompioRuntime::with_current(|rt| rt.spawn(future)).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `spawn_send` 功能单元测试。

    use compio::runtime::Runtime as CompioRuntime;

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn` 能把 future 投递到当前 compio 运行时，并
    /// 通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建 compio 运行时，在 `rt.block_on` 中调用 `Runtime::spawn`，
    /// await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn spawn_returns_output() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let handle = Runtime::spawn(async { 6 * 7 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
