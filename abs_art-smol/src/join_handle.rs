//! [`JoinHandle`] / [`JoinError`]：对 smol 任务句柄的薄包装。

use core::{
    convert::Infallible,
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use abs_art::{runtime::TrJoinHandle, TrAsyncRuntime};

use crate::Runtime;

/// 包装 `smol::Task<T>`；await 它以获取 `Result<T, JoinError>`。
///
/// smol 的任务不会「失败」（`smol::Task` 直接产出 `T`），因此
/// [`JoinError`] 实际是不可达的（`Infallible`）。
///
/// 对于 `spawn_local` 投递的任务，句柄还会持有对应的
/// [`LocalExecutor`](smol::LocalExecutor)，并在每次 poll 时驱动它——
/// 因为 smol 2.x 的 `block_on`（`async_io::block_on`）只轮询给定的 future，
/// 不会驱动本地执行器，本地任务必须由宿主显式 tick 才会推进。
pub struct JoinHandle<T> {
    inner: smol::Task<T>,
    /// `spawn_local` 专用：驱动本地任务的执行器；普通 `spawn` 为 `None`。
    local_ex: Option<smol::LocalExecutor<'static>>,
}

impl<T> TrJoinHandle<T> for JoinHandle<T>
where
    T: 'static,
{
    type JoinErr = JoinError;
}

/// `Runtime` 的句柄类型与能力无关：任何 `CAPS` 都使用同一个 `JoinHandle`。
impl<const CAPS: usize> TrAsyncRuntime for Runtime<CAPS> {
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn about() -> abs_art::Runtime {
        abs_art::Runtime::Smol
    }
}

impl<T> From<smol::Task<T>> for JoinHandle<T> {
    #[inline]
    fn from(task: smol::Task<T>) -> Self {
        JoinHandle {
            inner: task,
            local_ex: None,
        }
    }
}

impl<T> JoinHandle<T> {
    /// 从本地执行器投递的任务构造句柄，并携带执行器用于后续驱动。
    pub(crate) fn from_local(
        task: smol::Task<T>,
        ex: smol::LocalExecutor<'static>,
    ) -> Self {
        JoinHandle {
            inner: task,
            local_ex: Some(ex),
        }
    }
}

impl<T> Future for JoinHandle<T>
where
    T: 'static,
{
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // 若是本地任务，先驱动一次本地执行器，让任务有机会推进/完成。
        if let Some(ex) = &this.local_ex {
            while ex.try_tick() {}
        }

        // smol 的 Task（async-task）是 Unpin，可以直接投影。
        Pin::new(&mut this.inner).poll(cx).map(Ok)
    }
}

/// smol 任务的 join 错误。
///
/// smol 的 `Task` 没有失败的概念，因此该类型不可构造（包装 `Infallible`）。
pub struct JoinError(Infallible);

impl fmt::Debug for JoinError {
    #[allow(unreachable_code)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for JoinError {
    #[allow(unreachable_code)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl core::error::Error for JoinError {}
