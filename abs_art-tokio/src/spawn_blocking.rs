//! `spawn_blocking`：把阻塞函数投递到 tokio 的阻塞线程池。

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::TrSpawnBlocking;

impl Runtime {
    /// 把阻塞函数 `f` 投递到 tokio 的阻塞线程池，返回 [`JoinHandle`]。
    pub fn spawn_blocking<F, T>(f: F) -> JoinHandle<T>
    where
        Self: TrSpawnBlocking<F, T>,
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::spawn_blocking(f).into()
    }
}

impl<F, T> TrSpawnBlocking<F, T> for Runtime
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    type JoinHandle = JoinHandle<T> where T: 'static;

    fn spawn_blocking(f: F) -> JoinHandle<T> {
        Runtime::spawn_blocking(f)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端的 `spawn_blocking` 功能单元测试。

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn_blocking` 能把阻塞函数投递到阻塞线程池，
    /// 并通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建多线程 tokio 运行时，在 `rt.block_on` 中调用
    /// `Runtime::spawn_blocking` 执行一个简单的同步计算，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(40 + 2 == 42)`。
    #[test]
    fn spawn_blocking_returns_output() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();

        let out = rt.block_on(async {
            let handle = Runtime::spawn_blocking(|| 40 + 2);
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
