//! `time`：计时能力——周期源（[`TrInterval`]）与超时（[`TrTime`]）。
//!
//! 实现基于 smol 的 `Timer`（即 `async_io::Timer`，由 async-io 的反应器驱动）：
//!
//! - 一次性睡眠：[`TrDelay::Delay`] = `UnitFuture<smol::Timer>`（在 `delay.rs` 里给出）；
//! - 周期源：[`Interval`]——**自建**，因为 `async_io::Timer::interval` 的首次 tick
//!   落在「一个周期之后」，与本 crate 的契约（首次**立即**）不符；
//! - `tick` 的 future：[`Tick`]。
//!
//! 超时不必在这里实现：[`TrTime::timeout`] 是 trait 的默认方法，返回 `abs_art` 的
//! [`Timeout`](abs_art::Timeout)。
//!
//! 与 `delay` 共用 `delay` feature：两者要的是同一个驱动器（async-io 的反应器）。

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use abs_art::{HasDelay, TrDelay, TrInterval, TrTime, UnitFuture};

use crate::Runtime;

/// 本后端的周期源（[`TrTime::Interval`] 的具体类型）。
///
/// 内部机关是 `smol::Timer::interval(period)`：它在**构造时**就把时刻锚定为
/// `now + period`，此后每响一次把时刻 `+ period`（见 `async_io` 的
/// `Stream for Timer`），因此天然是「锚定」而非「上一觉之后再等一个 period」。
/// 我们只需把它的第一次响应当作「第二次 tick」——第一次 tick 由 [`Tick`] 直接放行。
#[derive(Debug)]
pub struct Interval {
    first_: bool,
    inner_: smol::Timer,
}

/// 本后端 `tick(&mut self)` 的 future（[`TrInterval::Tick`] 的具体类型）。
#[derive(Debug)]
pub struct Tick<'a> {
    interval_: &'a mut Interval,
}

impl Future for Tick<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // `Tick` 只含 `&mut Interval`，因此是 `Unpin`，`get_mut` 可用。
        // 注意要**重借**（`&mut *…`）：直接把 `&mut` 字段取出来会试图把它移出 `self`。
        let interval = &mut *self.get_mut().interval_;
        if interval.first_ {
            interval.first_ = false;
            return Poll::Ready(());
        }
        // `smol::Timer` 是 `Unpin`：安全地借用轮询（它同时是 `Future<Output = Instant>`，
        // 到点即就绪，并按周期重挂；返回值按契约丢弃）。
        Pin::new(&mut interval.inner_).poll(cx).map(|_| ())
    }
}

impl TrInterval for Interval {
    type Tick<'a> = Tick<'a>;

    fn tick(&mut self) -> Self::Tick<'_> {
        Tick { interval_: self }
    }
}

impl<const CAPS: usize> TrTime for Runtime<CAPS>
where
    [(); CAPS]: HasDelay,
{
    type Interval = Interval;

    fn interval(period: Duration) -> Self::Interval {
        smol_interval_(period)
    }
}

/// 让**本地作用域值**也承载一次性睡眠（[`TrDelay`]）与周期源（[`TrTime`]）。
///
/// 业务库手上只有作用域值（`S: TrLocalScope`），补上这两格之后写 `S: TrTime`
/// 一个约束就够，不必再引入第二个类型参数。smol 的计时源来自全局反应器，
/// 因此本实现**只借类型**、不读任何字段。
#[cfg(feature = "local_scope")]
impl TrDelay for crate::LocalScope {
    type Delay = UnitFuture<smol::Timer>;

    fn delay(duration: Duration) -> Self::Delay {
        UnitFuture::new(smol::Timer::after(duration))
    }
}

#[cfg(feature = "local_scope")]
impl TrTime for crate::LocalScope {
    type Interval = Interval;

    fn interval(period: Duration) -> Self::Interval {
        smol_interval_(period)
    }
}

/// 自建周期源：首次 `tick` 立即，其后锚定在构造时刻、每 `period` 一响。
///
/// # Panics
///
/// `period` 为零时 panic。**必须**自己断言：`async_io::Timer::interval(0)` 不会
/// panic，而是变成一个「每个时刻都立即到点」的忙循环。
fn smol_interval_(period: Duration) -> Interval {
    assert!(period > Duration::ZERO, "`period` must be non-zero.");
    Interval {
        first_: true,
        inner_: smol::Timer::interval(period),
    }
}

#[cfg(test)]
mod tests {
    //! smol 后端的 `TrTime` 单测。
    //!
    //! 跨三后端的一致性契约由 `abs_art-smoke` 的 `time_contract` 用例负责；这里只
    //! 钉住「本后端能不能跑起来」这几条。

    use std::time::Instant;

    use super::*;

    /// 目的：验证 `delay` 能真正睡到（async-io 的反应器被驱动）。
    ///
    /// 实施策略：`smol::block_on` 里 await 1 ms 的 `delay`，量测实际耗时。
    ///
    /// 通过依据：正常返回且耗时 `>= 1 ms`。
    #[test]
    fn delay_completes_and_waits_at_least_the_duration() {
        smol::block_on(async {
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
    /// 通过依据：耗时远小于周期（这里取 `< 1 秒`）。若照搬
    /// `async_io::Timer::interval` 的首次语义，本用例会等到 5 秒后才返回。
    #[test]
    fn interval_first_tick_is_immediate() {
        smol::block_on(async {
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

    /// 目的：验证自建周期源「锚定」而非「每响之后再等一个周期」。
    ///
    /// 实施策略：周期取 100 ms；第一次 tick 立即，此后先故意耗掉 150 ms 再第二次
    /// tick，量测第二次 tick 的耗时。
    ///
    /// 通过依据：如果实现是「上一觉之后再等 period」，第二次 tick 要再等 100 ms
    /// （总 250 ms）；锚定实现则在构造后 100 ms 处就该到点，因此耗时应当**远小于
    /// 100 ms**（这里取 `< 50 ms`）。
    #[test]
    fn interval_is_anchored_to_its_creation_instant() {
        smol::block_on(async {
            let mut period =
                <Runtime<{ crate::FULL }> as TrTime>::interval(Duration::from_millis(100));
            period.tick().await; // 立即
            smol::Timer::after(Duration::from_millis(150)).await;
            let started = Instant::now();
            period.tick().await;
            assert!(
                started.elapsed() < Duration::from_millis(50),
                "锚定实现不该再等满一个周期"
            );
        });
    }

    /// 目的：验证「睡眠 future 是 `Send`」在**编译期可见**。
    ///
    /// 实施策略：编译期断言 `<Runtime<{FULL}> as TrDelay>::Delay: Send`。
    ///
    /// 通过依据：编译通过即为通过——把返回类型从 `impl Future`（RPITIT）换成
    /// **关联类型**之后，自动 trait 能出现在调用方的约束里。
    #[test]
    fn delay_future_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<<Runtime<{ crate::FULL }> as TrDelay>::Delay>();
    }

    /// 目的：验证零周期在构造点被拒绝（契约第 5 条）。
    ///
    /// 实施策略：`#[should_panic]` 捕获 `interval(Duration::ZERO)`——async-io 自身
    /// **不**断言（会退化成忙循环），因此本后端必须自己挡。
    ///
    /// 通过依据：panic 文案含 "non-zero"。
    #[test]
    #[should_panic(expected = "non-zero")]
    fn interval_rejects_zero_period() {
        let _ = <Runtime<{ crate::FULL }> as TrTime>::interval(Duration::ZERO);
    }
}
