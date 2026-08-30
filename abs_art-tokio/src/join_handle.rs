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

    /// tokio 没有原生的 `detach` 方法；官方文档明确「drop `JoinHandle` 即
    /// detach」——任务继续在后台运行，只是失去等待它的句柄。因此这里直接
    /// 丢弃包装（内部持有的 `tokio::task::JoinHandle` 随之 drop，触发 detach
    /// 语义），与 tokio 一致。
    ///
    /// 注意：**不能**用 `abort()` 实现——abort 是取消任务，detach 是让任务
    /// 继续跑完，两者语义相反。
    fn detach(self) {
        drop(self);
    }
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

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端 `TrJoinHandle::detach` 的单元测试。

    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use abs_art::TrJoinHandle;

    use crate::Runtime;

    /// 目的：验证 `detach` 后任务仍在后台运行（tokio 的 drop 即 detach 语义）。
    ///
    /// 实施策略：spawn 一个设置 `AtomicBool` 的任务，`detach` 句柄（不再
    /// await），然后在外层 `block_on` 里循环 `yield_now` 直到标志被置位。
    ///
    /// 通过依据：标志最终为 `true`——若 detach 错误地取消/等待了任务，
    /// 循环会因超时断言失败；若 detach 正常工作，后台 worker 线程会执行
    /// 任务并置位。
    #[test]
    fn detach_keeps_task_running() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();

        rt.block_on(async {
            let handle = Runtime::spawn(async move {
                tokio::task::yield_now().await;
                f.store(true, Ordering::SeqCst);
            });
            handle.detach();

            // 句柄已消费，只能靠后台任务自己置位
            let mut spins = 0;
            while !flag.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
                spins += 1;
                assert!(spins < 1_000_000, "detach 后任务未推进");
            }
        });

        assert!(flag.load(Ordering::SeqCst));
    }
}
