//! `spawn_send`：把任务投递到 tokio 的全局工作窃取队列。

use core::future::Future;

use crate::{Runtime, join_handle::JoinHandle};
use abs_art::{HasSpawnSend, TrSpawnSend};

impl<const CAPS: usize> TrSpawnSend for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnSend,
{
    type JoinHandle<T>
        = JoinHandle<T>
    where
        T: 'static;

    fn spawn<F>(&self, future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        self.handle_.spawn(future).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端的 `spawn_send` 功能单元测试。

    use abs_art::TrSpawnSend;

    /// 目的：验证 `Runtime::spawn` 能把 future 投递到 tokio 全局队列，并通过
    /// 返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建多线程 tokio 运行时，在 `rt.block_on` 中构造运行时值并调用
    /// 其 `spawn`，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn spawn_returns_output() {
        let rt = tokio::runtime::Builder::new_multi_thread().build().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let handle = value.spawn(async { 6 * 7 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
