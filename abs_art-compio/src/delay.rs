//! `delay`：睡眠与延迟执行。

use core::time::Duration;

use compio::runtime::Runtime as CompioRuntime;

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::TrDelay;

/// 异步地睡眠 `duration`。
pub async fn sleep(duration: Duration) {
    compio::runtime::time::sleep(duration).await
}

/// 在 `interval` 之后执行 `f`，返回 [`JoinHandle`]。
///
/// `f` 会被投递到当前 compio 运行时上运行（compio 的运行时是线程本地的，
/// 因此这里不需要 `Send`）。
pub fn delayed<X, F>(interval: Duration, f: F) -> JoinHandle<X>
where
    X: 'static,
    F: FnOnce() -> X + 'static,
{
    CompioRuntime::with_current(|rt| {
        rt.spawn(async move {
            compio::runtime::time::sleep(interval).await;
            f()
        })
    })
    .into()
}

impl TrDelay for Runtime {
    /// 返回一个等待 `duration` 之后完成的 future。
    fn delay(duration: Duration) -> impl Future<Output = ()> {
        compio::runtime::time::sleep(duration)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `delay` 功能单元测试。

    use super::*;

    use compio::runtime::Runtime as CompioRuntime;

    /// 目的：验证 `sleep` 在 compio 运行时内可以正常完成（time driver 被驱动）。
    ///
    /// 实施策略：创建 compio 运行时，在 `rt.block_on` 中 await 一个 1ms 的
    /// `sleep`。
    ///
    /// 通过依据：`sleep` 正常返回且没有 panic；若 time driver 未被驱动，future
    /// 将永远 pending，`rt.block_on` 无法返回（测试挂死）。
    #[test]
    fn sleep_completes() {
        let rt = CompioRuntime::new().unwrap();

        rt.block_on(async { sleep(Duration::from_millis(1)).await });
    }

    /// 目的：验证 `delayed` 在指定间隔之后执行闭包并返回其结果。
    ///
    /// 实施策略：在 `rt.block_on` 中调用 `delayed`，await 其 JoinHandle 取回
    /// 闭包的返回值。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn delayed_runs_after_interval() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            delayed(Duration::from_millis(1), || 6 * 7)
                .await
                .unwrap()
        });

        assert_eq!(out, 42);
    }
}
