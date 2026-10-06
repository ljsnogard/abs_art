//! `spawn_blocking`：把阻塞函数投递到 tokio 的阻塞线程池。

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::{HasSpawnBlocking, TrSpawnBlocking};

impl<const CAPS: usize> TrSpawnBlocking for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnBlocking,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn spawn_blocking<F, T>(&self, f: F) -> Self::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.handle_.spawn_blocking(f).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端的 `spawn_blocking` 功能单元测试。

    use abs_art::TrSpawnBlocking;

    /// 目的：验证 `Runtime::spawn_blocking` 能把阻塞函数投递到阻塞线程池，
    /// 并通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建多线程 tokio 运行时，在 `rt.block_on` 中构造运行时值并调用
    /// 其 `spawn_blocking` 执行一个简单的同步计算，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(40 + 2 == 42)`。
    #[test]
    fn spawn_blocking_returns_output() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let handle = value.spawn_blocking(|| 40 + 2);
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
