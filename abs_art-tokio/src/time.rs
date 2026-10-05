//! `time`：计时能力——周期源（[`TrInterval`]）与超时（[`TrTime`]）。
//!
//! 实现基于 tokio 的 `time` 驱动。三样东西都是**具体类型**（不是 `impl Future`）：
//!
//! - 一次性睡眠：[`TrDelay::Delay`] = `tokio::time::Sleep`（在 `delay.rs` 里给出）；
//! - 周期源：[`Interval`]；
//! - `tick` 的 future：[`Tick`]——tokio 的 `Interval::tick` 是 `async fn`（不透明、
//!   不可命名），但它另给了 `pub fn poll_tick`，于是这个包装既具体又**完全安全**。
//!
//! 超时不必在这里实现：[`TrTime::timeout`] 是 trait 的默认方法，返回 `abs_art` 的
//! [`Timeout`](abs_art::Timeout)。
//!
//! 与 `delay` 共用 `delay` feature：两者要的是同一个运行时 feature（tokio 的
//! `time`），没有拆开的必要。

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use abs_art::{HasDelay, TrDelay, TrInterval, TrTime};
use tokio::time::MissedTickBehavior;

use crate::Runtime;

/// 本后端的周期源（[`TrTime::Interval`] 的具体类型）。
#[derive(Debug)]
pub struct Interval {
    inner_: tokio::time::Interval,
}

/// 本后端 `tick(&mut self)` 的 future（[`TrInterval::Tick`] 的具体类型）。
#[derive(Debug)]
pub struct Tick<'a> {
    interval_: &'a mut tokio::time::Interval,
}

impl Future for Tick<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // `&mut Interval` 是 `Unpin`，因此 `get_mut` 可用，无需任何 pin 投影技巧。
        self.get_mut().interval_.poll_tick(cx).map(|_| ())
    }
}

impl TrInterval for Interval {
    type Tick<'a> = Tick<'a>;

    fn tick(&mut self) -> Self::Tick<'_> {
        Tick {
            interval_: &mut self.inner_,
        }
    }
}

impl<const CAPS: usize> TrTime for Runtime<CAPS>
where
    [(); CAPS]: HasDelay,
{
    type Interval = Interval;

    fn interval(period: Duration) -> Self::Interval {
        tokio_interval_(period)
    }
}

/// 让**本地作用域值**也承载一次性睡眠（[`TrDelay`]）与周期源（[`TrTime`]）。
///
/// 业务库手上只有作用域值（`S: TrLocalScope`），补上这两格之后写 `S: TrTime`
/// 一个约束就够，不必再引入第二个类型参数。计时源来自运行时的线程本地上下文，
/// 不来自作用域值本身，因此本实现**只借类型**、不读任何字段。
#[cfg(feature = "local_scope")]
impl TrDelay for crate::LocalScope {
    type Delay = tokio::time::Sleep;

    fn delay(duration: Duration) -> Self::Delay {
        tokio::time::sleep(duration)
    }
}

#[cfg(feature = "local_scope")]
impl TrTime for crate::LocalScope {
    type Interval = Interval;

    fn interval(period: Duration) -> Self::Interval {
        tokio_interval_(period)
    }
}

/// 构造 tokio 的周期源：首次 tick 立即完成，进度锚定在构造时刻。
///
/// tokio 的 `interval` 缺省是 `MissedTickBehavior::Burst`（落后时把错过的时刻连续
/// 补出来）；这里显式取 `Skip`，与 compio 的相位对齐语义一致。契约只要求
/// 「锚定 + 首次立即」，落后行为属于各后端可自行选择的部分，见 `abs_art::time` 文档。
fn tokio_interval_(period: Duration) -> Interval {
    let mut inner_ = tokio::time::interval(period);
    inner_.set_missed_tick_behavior(MissedTickBehavior::Skip);
    Interval { inner_ }
}

#[cfg(test)]
mod tests {
    //! tokio 后端的 `TrTime` 单测。
    //!
    //! 跨三后端的一致性契约由 `abs_art-smoke` 的 `time_contract` 用例负责；这里只
    //! 钉住「本后端能不能跑起来」这几条。

    use std::time::Instant;

    use super::*;

    /// 建一个开着 time 驱动的 tokio 运行时。
    fn rt_() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("建 tokio 运行时")
    }

    /// 目的：验证 `delay` 能真正睡到（time 驱动被驱动）。
    ///
    /// 实施策略：在一个 tokio 运行时里 await 1 ms 的 `delay`，量测实际耗时。
    ///
    /// 通过依据：正常返回且耗时 `>= 1 ms`；若 time 驱动没被驱动，本用例会挂死
    /// （由外层 `rt.block_on` 不返回体现）。
    #[test]
    fn delay_completes_and_waits_at_least_the_duration() {
        let rt = rt_();
        rt.block_on(async {
            let started = Instant::now();
            <Runtime<{ crate::FULL }> as TrDelay>::delay(Duration::from_millis(1)).await;
            assert!(
                started.elapsed() >= Duration::from_millis(1),
                "delay 不该提前返回"
            );
        });
    }

    /// 目的：验证周期源的**第一次** tick 立即完成（契约第 2 条）。
    ///
    /// 实施策略：取一个 5 秒的周期，量测第一次 tick 的耗时。
    ///
    /// 通过依据：耗时远小于周期（这里取 `< 1 秒`）——若首次 tick 被推迟一个周期，
    /// 本用例会等到 5 秒后超时才结束。
    #[test]
    fn interval_first_tick_is_immediate() {
        let rt = rt_();
        rt.block_on(async {
            let mut period =
                <Runtime<{ crate::FULL }> as TrTime>::interval(Duration::from_secs(5));
            let started = Instant::now();
            period.tick().await;
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "第一次 tick 应当立即完成"
            );
        });
    }

    /// 目的：验证「睡眠 future 是 `Send`」在**编译期可见**。
    ///
    /// 实施策略：编译期断言 `<Runtime<{FULL}> as TrDelay>::Delay: Send`。
    ///
    /// 通过依据：编译通过即为通过。这正是把返回类型从 `impl Future`（RPITIT）换成
    /// **关联类型**的收益——自动 trait 现在能出现在调用方的约束里，而不再被不透明
    /// 类型挡住。
    #[test]
    fn delay_future_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<<Runtime<{ crate::FULL }> as TrDelay>::Delay>();
    }

    /// 目的：验证零周期在构造点被拒绝（契约第 5 条）。
    ///
    /// 实施策略：`#[should_panic]` 捕获 `interval(Duration::ZERO)`——tokio 自身
    /// 在 `interval(0)` 上就会 panic。
    ///
    /// 通过依据：panic 文案含 "zero"。
    #[test]
    #[should_panic(expected = "zero")]
    fn interval_rejects_zero_period() {
        let _ = <Runtime<{ crate::FULL }> as TrTime>::interval(Duration::ZERO);
    }
}
