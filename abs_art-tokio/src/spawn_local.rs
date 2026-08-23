//! `spawn_local`：把任务投递到 tokio 的线程本地队列。

use core::future::Future;

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::{FULL, HasSpawnLocal, TrSpawnLocal};

impl Runtime<FULL> {
    /// 把 `future` 投递到 tokio 的线程本地队列，返回 [`JoinHandle`]。
    ///
    /// 必须在 `tokio::task::LocalSet` 的上下文内调用（例如在
    /// `LocalSet::block_on` 或 `LocalSet::run_until` 内部），否则
    /// `tokio::task::spawn_local` 会 panic。
    pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
    where
        Self: TrSpawnLocal<F>,
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        tokio::task::spawn_local(future).into()
    }
}

impl<F, const CAPS: usize> TrSpawnLocal<F> for Runtime<CAPS>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
    [(); CAPS]: HasSpawnLocal,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn spawn_local(future: F) -> Self::JoinHandle<F::Output> {
        tokio::task::spawn_local(future).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端的 `spawn_local` 功能单元测试。

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn_local` 能在 `LocalSet` 上下文中投递任务并
    /// 通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建 current_thread tokio 运行时，用 `LocalSet::block_on`
    /// 进入本地任务上下文，在其中调用 `Runtime::spawn_local` 并 await 其
    /// JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`；若 `spawn_local` 没有
    /// 在 LocalSet 上下文中运行，tokio 会 panic，测试失败。
    #[test]
    fn spawn_local_returns_output() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        let out = local.block_on(&rt, async {
            let handle = Runtime::spawn_local(async { 6 * 7 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
