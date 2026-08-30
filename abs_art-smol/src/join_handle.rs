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

    /// smol 有原生 `Task::detach`（底层 async-task：置 detached 标志后 forget），
    /// 任务继续在全局执行器的后台线程上运行，完成后输出被丢弃。
    ///
    /// **不能**靠 drop 实现：async-task 的 `Task` 在 drop 时会 `set_canceled()`
    /// 取消任务——必须显式调用原生 `detach`。
    ///
    /// 限制：`spawn_local` 投递的本地任务，其本地执行器随本句柄存活（句柄
    /// poll 时驱动执行器）；detach 消费句柄后执行器随之销毁，任务无法继续
    /// 被驱动（等同取消）。因此 smol 后端的 `detach` 只对 `spawn`（全局执行器）
    /// 任务有完整语义。
    fn detach(self) {
        self.inner.detach();
        // 部分移动：inner 已消费；local_ex（若有）随函数结束被 drop，
        // 本地执行器销毁 → 本地任务无法继续推进（见上面的限制说明）
    }
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

#[cfg(test)]
mod tests {
    //! 针对 smol 后端 `TrJoinHandle::detach` 的单元测试。

    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    use abs_art::TrJoinHandle;

    use crate::Runtime;

    /// 目的：验证 `detach` 后任务在 smol 全局执行器的后台线程上继续运行。
    ///
    /// 实施策略：用 `Runtime::spawn`（全局执行器，由 smol 的后台线程驱动）
    /// 投递一个设置 `AtomicBool` 的任务，`detach` 句柄（不 await），然后轮询
    /// 标志直到置位（带期限）。
    ///
    /// 通过依据：标志在期限内被置位——若实现错误地用 drop（async-task 的
    /// drop 会 `set_canceled` 取消任务），任务永远不会执行，测试超时失败。
    #[test]
    fn detach_keeps_task_running() {
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();

        smol::block_on(async {
            let handle = Runtime::spawn(async move {
                smol::future::yield_now().await;
                f.store(true, Ordering::SeqCst);
            });
            handle.detach();
        });

        // 全局执行器由后台线程驱动，detach 的任务应该已经/即将置位
        let deadline = Instant::now() + Duration::from_secs(5);
        while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(flag.load(Ordering::SeqCst), "detach 后任务未在后台运行");
    }
}
