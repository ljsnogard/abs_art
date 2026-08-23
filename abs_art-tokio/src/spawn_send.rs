//! `spawn_send`：把任务投递到 tokio 的全局工作窃取队列。

use core::future::Future;

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::{FULL, HasSpawnSend, TrSpawnSend};

impl Runtime<FULL> {
    /// 把 `future` 投递到 tokio 的全局工作队列，返回 [`JoinHandle`]。
    pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
    where
        Self: TrSpawnSend<F>,
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        tokio::task::spawn(future).into()
    }
}

impl<F, const CAPS: usize> TrSpawnSend<F> for Runtime<CAPS>
where
    F: Future + Send + 'static,
    <F as Future>::Output: Send + 'static,
    [(); CAPS]: HasSpawnSend,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn spawn(future: F) -> Self::JoinHandle<F::Output> {
        tokio::task::spawn(future).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端的 `spawn_send` 功能单元测试。

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn` 能把 future 投递到 tokio 全局队列，并通过
    /// 返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建多线程 tokio 运行时，在 `rt.block_on` 中调用
    /// `Runtime::spawn`，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn spawn_returns_output() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();

        let out = rt.block_on(async {
            let handle = Runtime::spawn(async { 6 * 7 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
