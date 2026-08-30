//! [`JoinHandle`] / [`JoinError`]：对 compio 任务句柄的薄包装。

use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use abs_art::{runtime::TrJoinHandle, TrAsyncRuntime};

use crate::Runtime;

/// 包装 `compio::runtime::JoinHandle<T>`；await 它以获取 `Result<T, JoinError>`。
pub struct JoinHandle<T> {
    inner: compio::runtime::JoinHandle<T>,
}

/// `Runtime` 的句柄类型与能力无关：任何 `CAPS` 都使用同一个 `JoinHandle`。
impl<const CAPS: usize> TrAsyncRuntime for Runtime<CAPS> {
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn about() -> abs_art::Runtime {
        abs_art::Runtime::Compio
    }
}

impl<T> TrJoinHandle<T> for JoinHandle<T>
where
    T: 'static,
{
    type JoinErr = JoinError;

    /// compio 有原生 `JoinHandle::detach`：丢弃任务句柄而不取消任务，任务
    /// 继续在当前运行时的工作队列里推进，完成后输出被丢弃。
    ///
    /// **不能**靠 drop 实现：compio 的 `JoinHandle` 在 drop 时会
    /// `cancel(true)` 取消任务——必须显式调用原生 `detach`。
    fn detach(self) {
        self.inner.detach();
    }
}

impl<T> From<compio::runtime::JoinHandle<T>> for JoinHandle<T> {
    #[inline]
    fn from(handle: compio::runtime::JoinHandle<T>) -> Self {
        JoinHandle { inner: handle }
    }
}

impl<T> Future for JoinHandle<T>
where
    T: 'static,
{
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // compio 的 JoinHandle 是 Unpin，可以直接投影。
        let this = self.get_mut();
        Pin::new(&mut this.inner)
            .poll(cx)
            .map(|res| res.map_err(JoinError))
    }
}

/// compio 任务的 join 错误（包装 `compio::runtime::JoinError`）。
pub struct JoinError(compio::runtime::JoinError);

impl JoinError {
    /// 取出内部的 compio join 错误。
    pub fn into_inner(self) -> compio::runtime::JoinError {
        self.0
    }
}

impl core::convert::AsRef<compio::runtime::JoinError> for JoinError {
    fn as_ref(&self) -> &compio::runtime::JoinError {
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
    //! 针对 compio 后端 `TrJoinHandle::detach` 的单元测试。

    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use compio::runtime::Runtime as CompioRuntime;

    use abs_art::TrJoinHandle;

    use crate::Runtime;

    /// 目的：验证 `detach` 后任务仍在 compio 运行时的工作队列里推进。
    ///
    /// 实施策略：`Runtime::spawn` 一个设置 `AtomicBool` 的任务，`detach`
    /// 句柄（不 await），然后 `sleep` 让出——compio 的 `block_on` 在等待
    /// 期间会 tick 运行时队列，detach 的任务因此有机会执行并置位。
    ///
    /// 通过依据：标志在 `block_on` 返回前被置位——若 detach 实现错误地触发
    /// 了取消（compio JoinHandle 的 drop 会 cancel），标志永远不会置位。
    #[test]
    fn detach_keeps_task_running() {
        let rt = CompioRuntime::new().unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();

        rt.block_on(async {
            let handle = Runtime::spawn(async move {
                f.store(true, Ordering::SeqCst);
            });
            handle.detach();
            // 让出：compio 的 block_on 在等待期间会驱动运行时队列
            compio::runtime::time::sleep(std::time::Duration::from_millis(10)).await;
        });

        assert!(flag.load(Ordering::SeqCst), "detach 后任务未被调度执行");
    }
}
