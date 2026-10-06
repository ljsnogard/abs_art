//! `delay`：睡眠与延迟执行。
//!
//! 计时源是 smol 的 `Timer`（即 `async_io::Timer`，由 async-io 的进程级反应器
//! 驱动）。值化之后它挂在运行时**值**上：
//!
//! ```text
//! rt.delay(d)   —— 计时源取自这个值（与 rt.now() 同一时间基准，见 time.rs）
//! ```
//!
//! 与 `time` 模块共用 `delay` feature：两者要的是同一个驱动器（async-io 反应器）。

use core::time::Duration;

use abs_art::{HasDelay, TrDelay, UnitFuture};

use crate::{Runtime, join_handle::JoinHandle};

/// 异步地睡眠 `duration`。
///
/// 本自由函数不依赖运行时值：`smol::Timer` 用进程级反应器，任何位置都能 await。
pub async fn sleep(duration: Duration) {
    smol::Timer::after(duration).await;
}

/// 在 `interval` 之后执行 `f`（Send 版），返回 [`JoinHandle`]。
///
/// `f` 会被投递到 smol 的**进程级全局执行器**（运行在独立的后台线程上，见 crate
/// 文档），因此需要 `Send + 'static`；这条投递**与任何运行时值无关**。
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

impl<const CAPS: usize> TrDelay for Runtime<CAPS>
where
    [(); CAPS]: HasDelay,
{
    /// 本后端的睡眠 future 类型。
    ///
    /// `smol::Timer` 完成时返回**到期时刻**（`std::time::Instant`），而
    /// [`TrDelay::Delay`] 要求 `Output = ()`，因此用共享的 [`UnitFuture`] 包一层。
    type Delay = UnitFuture<smol::Timer>;

    /// 返回一个等待 `duration` 之后完成的 future。
    ///
    /// 计时源是进程级的 async-io 反应器；本值在这里的作用是**把「这个运行时的
    /// 计时能力」变成一次值方法调用**，从而与 [`TrClock::now`](abs_art::TrClock::now)
    /// 不可能指向别处。
    fn delay(&self, duration: Duration) -> Self::Delay {
        UnitFuture::new(smol::Timer::after(duration))
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `delay` 功能单元测试。

    use abs_art::{FULL, TrDelay};

    use super::*;

    /// 目的：验证 `sleep` 能正常完成（async-io 的 Timer 被驱动）。
    ///
    /// 手段：直接调用 `smol::block_on` await 一个 1ms 的 `sleep`。
    ///
    /// 判定：`sleep` 正常返回且没有 panic；若反应器未被驱动，本用例会挂死。
    #[test]
    fn sleep_completes() {
        smol::block_on(async { sleep(Duration::from_millis(1)).await });
    }

    /// 目的：验证 `delayed` 在指定间隔之后执行闭包并返回其结果。
    ///
    /// 手段：调用 `smol::block_on`，在其中调用 `delayed`，await 其 JoinHandle
    /// 取回闭包的返回值。
    ///
    /// 判定：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn delayed_runs_after_interval() {
        let out =
            smol::block_on(async { delayed(Duration::from_millis(1), || 6 * 7).await.unwrap() });

        assert_eq!(out, 42);
    }

    /// 目的：验证**值方法**形式的 `delay` 不早于 `duration` 返回。
    ///
    /// 手段：构造 `Runtime<{ FULL }>` 值，用 `rt.delay(1ms)` 得到睡眠 future，
    /// 在 `smol::block_on` 中 await 它并量测实际耗时。
    ///
    /// 判定：正常返回且耗时 `>= 1ms`；若计时源没有真正被驱动，本用例会挂死。
    #[test]
    fn value_delay_waits_at_least_the_duration() {
        let rt = Runtime::<{ FULL }>::current();
        let started = std::time::Instant::now();
        smol::block_on(rt.delay(Duration::from_millis(1)));
        assert!(
            started.elapsed() >= Duration::from_millis(1),
            "delay 不该提前返回"
        );
    }
}
