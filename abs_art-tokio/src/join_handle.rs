//! [`JoinHandle`] / [`JoinError`]：对 tokio 任务句柄的薄包装。

use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use abs_art::{runtime::TrJoinHandle, TrAsyncRuntime};

use crate::Runtime;

/// 包装 `tokio::task::JoinHandle<T>`；await 它以获取 `Result<T, JoinError>`。
pub struct JoinHandle<T> {
    inner: tokio::task::JoinHandle<T>,
}

/// `Runtime` 的句柄类型与能力无关：任何 `CAPS` 都使用同一个 `JoinHandle`。
impl<const CAPS: usize> TrAsyncRuntime for Runtime<CAPS> {
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn about() -> abs_art::Runtime {
        abs_art::Runtime::Tokio
    }
}

impl<T> From<tokio::task::JoinHandle<T>> for JoinHandle<T> {
    #[inline]
    fn from(handle: tokio::task::JoinHandle<T>) -> Self {
        JoinHandle { inner: handle }
    }
}

impl<T> TrJoinHandle<T> for JoinHandle<T>
where
    T: 'static,
{
    type JoinErr = JoinError;
}

impl<T> Future for JoinHandle<T>
where
    T: 'static,
{
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // tokio 的 JoinHandle 是 Unpin，可以直接投影。
        let this = self.get_mut();
        Pin::new(&mut this.inner)
            .poll(cx)
            .map(|res| res.map_err(JoinError))
    }
}

/// tokio 任务的 join 错误（包装 `tokio::task::JoinError`）。
pub struct JoinError(tokio::task::JoinError);

impl JoinError {
    /// 取出内部的 tokio join 错误。
    pub fn into_inner(self) -> tokio::task::JoinError {
        self.0
    }
}

impl core::convert::AsRef<tokio::task::JoinError> for JoinError {
    fn as_ref(&self) -> &tokio::task::JoinError {
        &self.0
    }
}

impl fmt::Debug for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl core::error::Error for JoinError {}
