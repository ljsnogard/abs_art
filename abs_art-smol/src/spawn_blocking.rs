//! `spawn_blocking`：把阻塞函数投递到 smol 的阻塞线程池。

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::TrSpawnBlocking;

impl Runtime {
    /// 把阻塞函数 `f` 投递到 smol 的阻塞线程池（`smol::unblock`），返回
    /// [`JoinHandle`]。
    pub fn spawn_blocking<F, T>(f: F) -> JoinHandle<T>
    where
        Self: TrSpawnBlocking<F, T>,
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        smol::unblock(f).into()
    }
}

impl<F, T> TrSpawnBlocking<F, T> for Runtime
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    type JoinHandle = JoinHandle<T> where F: 'static;

    fn spawn_blocking(f: F) -> JoinHandle<T> {
        smol::unblock(f).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `spawn_blocking` 功能单元测试。

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn_blocking` 能把阻塞函数投递到 smol 的阻塞
    /// 线程池，并通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：调用 `smol::block_on`，在其中调用 `Runtime::spawn_blocking`
    /// 执行一个简单的同步计算，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(40 + 2 == 42)`。
    #[test]
    fn spawn_blocking_returns_output() {
        let out = smol::block_on(async {
            let handle = Runtime::spawn_blocking(|| 40 + 2);
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
