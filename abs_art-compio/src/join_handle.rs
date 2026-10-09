//! [`JoinHandle`] / [`JoinError`]：对 compio 任务句柄的薄包装。

use core::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use abs_art::{TrAsyncRuntime, runtime::TrJoinHandle};

use crate::{CompioCaps_, Runtime};

/// 包装 `compio::runtime::JoinHandle<T>`；await 它以获取 `Result<T, JoinError>`。
pub struct JoinHandle<T> {
    inner: compio::runtime::JoinHandle<T>,
}

/// `Runtime` 的句柄类型与能力无关：任何 `CAPS` 都使用同一个 `JoinHandle`。
///
/// `about` 收 `&self`（值化后的形状）：身份由运行时值报告，而不是由类型报告。
/// [`TrAsyncRuntime`] 也是能力 trait，因此同样加上
/// [`CompioCaps_`] 门控：泛型代码里写 `R: TrAsyncRuntime`、实参写成
/// `Runtime<{SPAWN_SEND}>` 时会在这里报同一条人话错误，而不是拖到调用点。
impl<const CAPS: usize> TrAsyncRuntime for Runtime<CAPS>
where
    [(); CAPS]: CompioCaps_,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn about(&self) -> abs_art::RuntimeTag {
        abs_art::RuntimeTag::Compio
    }
}

impl<T> From<compio::runtime::JoinHandle<T>> for JoinHandle<T> {
    #[inline]
    fn from(handle: compio::runtime::JoinHandle<T>) -> Self {
        JoinHandle { inner: handle }
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

    use std::{cell::Cell, rc::Rc};

    use abs_art::{TrJoinHandle, TrLocalScope};
    use compio::runtime::Runtime as CompioRuntime;

    /// 目的：验证 `detach` 后任务仍在**运行时钉住的**本地队列里推进（compio 的队列由
    /// 运行时自己 tick）。
    ///
    /// 实施策略：经运行时值交出本地作用域，用 `spawn_local` 投递一个置位 `Rc<Cell<bool>>`
    /// 的任务，`detach` 句柄（不 await），再 `sleep` 让出——compio 的 `block_on` 在等待期间
    /// 会 tick 运行时队列，detach 的任务因此有机会执行并置位。
    ///
    /// 通过依据：标志在 `block_on` 返回前被置位——若 detach 实现错误地触发了取消（compio
    /// `JoinHandle` 的 drop 会 cancel），标志永远不会置位。
    #[test]
    fn detach_keeps_task_running() {
        let rt = CompioRuntime::new().unwrap();
        let flag = Rc::new(Cell::new(false));
        let f = flag.clone();

        rt.block_on(async {
            let scope = crate::current().local_scope();
            let handle = scope.spawn_local(async move {
                f.set(true);
            });
            handle.detach();
            // 让出：compio 的 block_on 在等待期间会驱动运行时队列
            compio::runtime::time::sleep(std::time::Duration::from_millis(10)).await;
        });

        assert!(flag.get(), "detach 后任务未被调度执行");
    }
}
