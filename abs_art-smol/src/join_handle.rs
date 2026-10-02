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
/// # 句柄不再持有本地执行器
///
/// v0.3 之前，本类型在 `spawn_local` 场景下还持有一个
/// [`LocalExecutor`](smol::LocalExecutor)，并在每次 poll 时顺带 tick 它——于是
/// 「本地任务能否推进」取决于「调用方有没有在 poll 句柄」，`detach()` 消费句柄
/// 还会连带销毁执行器、把任务当场取消。
///
/// 现在执行器归 [`LocalScope`](crate::LocalScope) 所有，本类型只是
/// `smol::Task` 的薄包装：是否能推进与句柄无关，`detach()` 也回到「任务继续跑、
/// 只是不要结果」的正常语义。
pub struct JoinHandle<T> {
    inner: smol::Task<T>,
}

impl<T> TrJoinHandle<T> for JoinHandle<T>
where
    T: 'static,
{
    type JoinErr = JoinError;

    /// smol 有原生 `Task::detach`（底层 async-task：置 detached 标志后 forget），
    /// 任务继续在它所属的执行器上运行，完成后输出被丢弃。
    ///
    /// **不能**靠 drop 实现：async-task 的 `Task` 在 drop 时会 `set_canceled()`
    /// 取消任务——必须显式调用原生 `detach`。
    ///
    /// 本地任务（`spawn_local` 投递）的执行器由
    /// [`LocalScope`](crate::LocalScope) 持有，因此这里 detach 之后任务照常推进。
    fn detach(self) {
        self.inner.detach();
    }
}

/// `Runtime` 的句柄类型与能力无关：任何 `CAPS` 都使用同一个 `JoinHandle`。
impl<const CAPS: usize> TrAsyncRuntime for Runtime<CAPS> {
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn about() -> abs_art::RuntimeTag {
        abs_art::RuntimeTag::Smol
    }
}

impl<T> From<smol::Task<T>> for JoinHandle<T> {
    #[inline]
    fn from(task: smol::Task<T>) -> Self {
        JoinHandle { inner: task }
    }
}

impl<T> Future for JoinHandle<T>
where
    T: 'static,
{
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // smol 的 Task（async-task）是 Unpin，可以直接投影。
        // 注意：这里**不再**顺带驱动任何执行器——驱动是 LocalScope 的职责。
        let this = self.get_mut();
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
