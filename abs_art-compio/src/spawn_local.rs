//! `spawn_local`：把任务投递到 compio 的线程本地队列。
//!
//! compio 的运行时本身就是线程本地的（`compio_runtime::Runtime` 不能跨线程
//! 发送），因此这里的 `spawn_local` 与 `spawn` 行为一致，只是放宽了
//! `Send` 约束。

use core::future::Future;

use compio::runtime::Runtime as CompioRuntime;

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::{FULL, HasSpawnLocal, TrSpawnLocal};

impl Runtime<FULL> {
    /// 把 `future` 投递到当前 compio 运行时的工作队列（线程本地），返回
    /// [`JoinHandle`]。
    pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
    where
        Self: TrSpawnLocal<F>,
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        CompioRuntime::with_current(|rt| rt.spawn(future)).into()
    }
}

impl<F, const CAPS: usize> TrSpawnLocal<F> for Runtime<CAPS>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
    [(); CAPS]: HasSpawnLocal,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    #[inline]
    fn spawn_local(future: F) -> JoinHandle<F::Output> {
        Runtime::spawn_local(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `spawn_local` 功能单元测试。

    use compio::runtime::Runtime as CompioRuntime;

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn_local` 能把 future 投递到当前 compio 运行时，
    /// 并通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建 compio 运行时，在 `rt.block_on` 中调用 `Runtime::spawn_local`
    /// await 其 JoinHandle。compio 的运行时是线程本地的，因此 `spawn_local`
    /// 与 `spawn` 等价。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn spawn_local_returns_output() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let handle = Runtime::spawn_local(async { 6 * 7 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
