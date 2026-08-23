//! `delay`：睡眠与延迟执行。

use core::time::Duration;

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::TrDelay;

/// 异步地睡眠 `duration`。
pub async fn sleep(duration: Duration) {
    smol::Timer::after(duration).await;
}

/// 在 `interval` 之后执行 `f`（Send 版），返回 [`JoinHandle`]。
///
/// `f` 会被投递到 smol 的全局执行器（运行在独立的后台线程上），因此需要
/// `Send + 'static`。
pub fn delayed<X, F>(interval: Duration, f: F) -> JoinHandle<X>
where
    X: Send + 'static,
    F: FnOnce() -> X + Send + 'static,
{
    smol::spawn(async move {
        smol::Timer::after(interval).await;
        f()
    })
    .into()
}

impl TrDelay for Runtime {
    /// 返回一个等待 `duration` 之后完成的 future。
    async fn delay(duration: Duration) {
        smol::Timer::after(duration).await;
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `delay` 功能单元测试。

    use super::*;

    /// 目的：验证 `sleep` 能正常完成（async-io 的 Timer 被驱动）。
    ///
    /// 实施策略：直接调用 `smol::block_on` await 一个 1ms 的 `sleep`。
    ///
    /// 通过依据：`sleep` 正常返回且没有 panic。
    #[test]
    fn sleep_completes() {
        smol::block_on(async { sleep(Duration::from_millis(1)).await });
    }

    /// 目的：验证 `delayed` 在指定间隔之后执行闭包并返回其结果。
    ///
    /// 实施策略：调用 `smol::block_on`，在其中调用 `delayed`，await 其
    /// JoinHandle 取回闭包的返回值。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn delayed_runs_after_interval() {
        let out = smol::block_on(async {
            delayed(Duration::from_millis(1), || 6 * 7)
                .await
                .unwrap()
        });

        assert_eq!(out, 42);
    }
}
