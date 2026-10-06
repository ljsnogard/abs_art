//! [`JoinHandle`] / [`JoinError`]：对 smol 任务句柄的薄包装。

use core::{
    convert::Infallible,
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use abs_art::{TrAsyncRuntime, runtime::TrJoinHandle};

use crate::Runtime;

/// 包装 `smol::Task<T>`；await 它以获取 `Result<T, JoinError>`。
///
/// smol 的任务不会「失败」（`smol::Task` 直接产出 `T`），因此
/// [`JoinError`] 实际是不可达的（`Infallible`）。
///
/// # 句柄不持有任何队列
///
/// 本类型只是 `smol::Task` 的薄包装，既**不**持有本地执行器，也**不**持有全局
/// 执行器（后者是进程级单例，见 crate 文档，任何值都持不了它）：
///
/// - 全局任务（[`TrSpawnSend::spawn`](abs_art::TrSpawnSend::spawn)）由 smol 的
///   后台线程驱动，能否推进与句柄无关；
/// - 本地任务（[`TrLocalScope::spawn_local`](abs_art::TrLocalScope::spawn_local)）
///   由**作用域值**（`LocalScope`）持有的 `LocalExecutor` 驱动，同样与句柄无关；
/// - `detach()` 因此回到「任务继续跑、只是不要结果」的正常语义。
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
    /// 本地任务（`spawn_local` 投递）的执行器由**作用域值**持有，因此这里 detach
    /// 之后任务照常推进（只要作用域还活着且仍被驱动）。
    fn detach(self) {
        self.inner.detach();
    }
}

/// `Runtime` 的句柄类型与能力无关：任何 `CAPS` 都使用同一个 `JoinHandle`。
impl<const CAPS: usize> TrAsyncRuntime for Runtime<CAPS> {
    type JoinHandle<T>
        = JoinHandle<T>
    where
        T: 'static;

    fn about(&self) -> abs_art::RuntimeTag {
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
        // 注意：这里**不**顺带驱动任何执行器——本地队列归作用域值所有。
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
    //! 针对 smol 后端句柄上报的身份与 `TrJoinHandle::detach` 的单元测试。

    use abs_art::{RuntimeTag, TrAsyncRuntime};

    use crate::Runtime;

    /// 目的：验证运行时**值**自报的身份是 [`RuntimeTag::Smol`]，且与固有方法
    /// `tag()` 一致。
    ///
    /// 手段：构造 `Runtime<{ FULL }>` 值，分别经 `TrAsyncRuntime::about`（值方法）
    /// 与固有 `tag()` 读取身份。
    ///
    /// 判定：两次读取都等于 `RuntimeTag::Smol`；若值化把身份搬到了错误的位置
    /// （例如仍在无 `self` 的关联函数上），本测试无法编译。
    #[test]
    fn value_reports_smol_identity() {
        let value = Runtime::<{ crate::FULL }>::current();
        assert_eq!(value.about(), RuntimeTag::Smol);
        assert_eq!(value.tag(), RuntimeTag::Smol);
    }

    /// 目的：验证 `detach` 后全局任务在 smol 全局执行器的后台线程上继续运行
    /// ——即「值钉不住全局队列」这条限制的可观测后果。
    ///
    /// 手段：用运行时值的 `spawn`（转发到 `smol::spawn` 的进程级执行器）投递一个
    /// 设置 `AtomicBool` 的任务，`detach` 句柄（不 await），**并把运行时值也
    /// drop 掉**，然后以 `std::thread::sleep` 轮询标志直到置位（带 5 秒期限）。
    ///
    /// 判定：标志在期限内被置位。若实现错误地用 drop（async-task 的 drop 会
    /// `set_canceled` 取消任务）实现 `detach`，任务永远不会执行；若任务其实归
    /// 运行时值所有，drop 值之后它也不该继续跑——两种情况都会超时失败。
    #[cfg(feature = "spawn_send")]
    #[test]
    fn detach_keeps_task_running_after_value_is_dropped() {
        use std::{
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            time::{Duration, Instant},
        };

        use abs_art::{TrJoinHandle, TrSpawnSend};

        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();

        {
            let value = crate::current();
            let handle = value.spawn(async move {
                smol::future::yield_now().await;
                f.store(true, Ordering::SeqCst);
            });
            handle.detach();
        } // 运行时值在此 drop：全局任务不受影响

        let deadline = Instant::now() + Duration::from_secs(5);
        while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(flag.load(Ordering::SeqCst), "detach 后任务未在后台运行");
    }
}
