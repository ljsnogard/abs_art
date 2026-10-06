//! 手动时钟上的 delay 与周期源。

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use abs_art::TrInterval;

use crate::{clock::ManualClock, instant::MockInstant};

/// 手动时钟上的 delay future（即 [`crate::ManualClockApi::Delay`]）。
///
/// 首次 poll 时把 waker 登记进时钟的到期表；时钟被推进到截止时刻后由时钟唤醒。
/// 重复 poll 只在 waker 变化时重新登记（避免到期表无限增长）。
pub struct MockDelay<I: MockInstant> {
    /// 时钟句柄。
    clock_: ManualClock<I>,
    /// 截止时刻（毫秒刻度）。
    deadline_millis_: u64,
    /// 上次登记的 waker。
    registered_: Option<Waker>,
}

impl<I: MockInstant> MockDelay<I> {
    /// 后端起构造：由 [`crate::ManualClockApi::sleep`] 调用。
    pub(crate) fn new_(clock: ManualClock<I>, deadline_millis: u64) -> Self {
        Self {
            clock_: clock,
            deadline_millis_: deadline_millis,
            registered_: None,
        }
    }

    /// 本 delay 的截止时刻（毫秒刻度）。
    #[must_use]
    pub fn deadline_millis(&self) -> u64 {
        self.deadline_millis_
    }
}

impl<I: MockInstant> Future for MockDelay<I> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // `MockDelay` 的所有字段都是 `Unpin`，因此可以安全地取 `&mut self`。
        let this = self.as_mut().get_mut();
        if this.clock_.now_millis_() >= this.deadline_millis_ {
            return Poll::Ready(());
        }
        let stale = this
            .registered_
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(cx.waker()));
        if stale {
            this.clock_
                .register_(this.deadline_millis_, cx.waker().clone());
            this.registered_ = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// 手动时钟上的周期源（即 [`crate::ManualClockApi::Interval`]）。
///
/// 与三个后端的契约一致：**首次 tick 立即完成**，此后每 `period` 一次。
pub struct MockInterval<I: MockInstant> {
    /// 时钟句柄。
    clock_: ManualClock<I>,
    /// 下一次 tick 的截止时刻（毫秒刻度）。
    next_millis_: u64,
    /// 周期（毫秒）。
    period_millis_: u64,
}

impl<I: MockInstant> MockInterval<I> {
    /// 后端起构造：由 [`crate::ManualClockApi::interval`] 调用。
    pub(crate) fn new_(clock: ManualClock<I>, period_millis: u64) -> Self {
        let next = clock.now_millis_();
        Self {
            clock_: clock,
            next_millis_: next,
            period_millis_: period_millis,
        }
    }

    /// 周期（毫秒）。
    #[must_use]
    pub fn period_millis(&self) -> u64 {
        self.period_millis_
    }
}

impl<I: MockInstant> TrInterval for MockInterval<I> {
    type Tick<'a>
        = MockDelay<I>
    where
        Self: 'a;

    fn tick(&mut self) -> Self::Tick<'_> {
        let tick = MockDelay::new_(self.clock_.clone(), self.next_millis_);
        self.next_millis_ = self.next_millis_.saturating_add(self.period_millis_);
        tick
    }
}

#[cfg(test)]
mod tests {
    //! [`MockDelay`] / [`MockInterval`] 的行为。

    use core::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
        time::Duration,
    };

    use abs_art::TrInterval;

    use crate::clock::{ManualClock, ManualClockApi};

    /// 目的：验证 delay 在时钟未推进时是 `Pending`、推进到点后变 `Ready`。
    ///
    /// 手段：手工 poll（用 `Waker::noop()`，不需要任何运行时）。
    ///
    /// 判断：初始 `Pending`；`advance` 到 1000ms 后 `Ready(())`。
    #[test]
    fn delay_completes_only_after_the_clock_reaches_it() {
        let clock = ManualClock::new();
        let mut delay = pin!(clock.sleep(Duration::from_millis(1_000)));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert!(matches!(delay.as_mut().poll(&mut cx), Poll::Pending));

        clock.advance_by(Duration::from_millis(999));
        assert!(matches!(delay.as_mut().poll(&mut cx), Poll::Pending));

        clock.advance_by(Duration::from_millis(1));
        assert!(matches!(delay.as_mut().poll(&mut cx), Poll::Ready(())));
    }

    /// 目的：验证零延时立即完成（`now() >= deadline` 的边界）。
    ///
    /// 手段：`sleep(Duration::ZERO)` 后直接 poll。
    ///
    /// 判断：`Ready(())`。
    #[test]
    fn zero_delay_is_immediately_ready() {
        let clock = ManualClock::new();
        let mut delay = pin!(clock.sleep(Duration::ZERO));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        assert!(matches!(delay.as_mut().poll(&mut cx), Poll::Ready(())));
    }

    /// 目的：验证周期源首次 tick 立即完成、之后每周期一次。
    ///
    /// 手段：取周期 100ms 的 interval，连续 poll 三次 tick（每次推进 100ms）。
    ///
    /// 判断：第一次立即 `Ready`；第二次在推进 100ms 后 `Ready`；第三次同理。
    #[test]
    fn interval_ticks_immediately_then_periodically() {
        let clock = ManualClock::new();
        let mut interval = clock.interval(Duration::from_millis(100));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        {
            let mut first = pin!(interval.tick());
            assert!(matches!(first.as_mut().poll(&mut cx), Poll::Ready(())));
        }

        {
            let mut second = pin!(interval.tick());
            assert!(matches!(second.as_mut().poll(&mut cx), Poll::Pending));
            clock.advance_by(Duration::from_millis(100));
            assert!(matches!(second.as_mut().poll(&mut cx), Poll::Ready(())));
        }

        {
            let mut third = pin!(interval.tick());
            assert!(matches!(third.as_mut().poll(&mut cx), Poll::Pending));
            clock.advance_by(Duration::from_millis(100));
            assert!(matches!(third.as_mut().poll(&mut cx), Poll::Ready(())));
        }
    }

    /// 目的：验证零周期被拒绝（与三后端 `interval` 契约一致）。
    ///
    /// 手段：`interval(Duration::ZERO)` 并捕获 panic。
    ///
    /// 判断：发生 panic。
    #[test]
    #[should_panic(expected = "`period` must be non-zero.")]
    fn zero_period_panics() {
        let clock = ManualClock::new();
        let _ = clock.interval(Duration::ZERO);
    }
}
