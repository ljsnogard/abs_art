//! 装饰器：把任意运行时值的**时间**换成手动时钟，其余能力原样委托。

use core::{fmt, future::Future, time::Duration};

use abs_art::{
    TrBlockOn, TrClock, TrDelay, TrMockClock, TrSpawnBlocking,
    TrSpawnSend, TrTime,
};

use crate::{
    advance::MockAdvance,
    clock::{ManualClock, ManualClockApi},
    instant::MillisInstant,
};

/// 把 `inner` 的时间换成 `clock` 的装饰器。
///
/// # 覆盖与委托
///
/// | 能力 | 来源 |
/// | --- | --- |
/// | [`TrDelay`] / [`TrClock`] / [`TrTime`] | **手动时钟**（因此「计时器与时刻同源」天然成立） |
/// | [`TrAsyncRuntime`] / [`TrSpawnSend`] / [`TrSpawnBlocking`] / [`TrBlockOn`] | 原样委托给 `inner` |
/// | 本地队列（`LocalScope`） | 不在本值上：从 `inner()` 拿到运行时值再取（见各后端文档） |
///
/// # Examples
///
/// ```
/// use core::time::Duration;
/// use abs_art::TrClock;
/// use abs_art_mock_clock::{ManualClock, ManualClockApi, ManualTime, MockInstant, TrMockClock};
///
/// # struct FakeRt_;
/// let clock = ManualClock::new();
/// let value = ManualTime::new(FakeRt_, clock.clone());
///
/// clock.advance_by(Duration::from_secs(5));
/// assert_eq!(value.now().as_millis(), 5_000);
/// assert_eq!(value.clock().now().as_millis(), 5_000);
/// ```
pub struct ManualTime<R, C: ManualClockApi = ManualClock<MillisInstant>> {
    /// 被装饰的运行时值。
    inner_: R,
    /// 手动时钟（时间能力的唯一来源）。
    clock_: C,
}

impl<R, C: ManualClockApi> ManualTime<R, C> {
    /// 用给定的手动时钟装饰 `inner`。
    pub fn new(inner: R, clock: C) -> Self {
        Self {
            inner_: inner,
            clock_: clock,
        }
    }

    /// 取被装饰的运行时值（用于取本地作用域等装饰器之外的能力）。
    pub fn inner(&self) -> &R {
        &self.inner_
    }

    /// 取手动时钟句柄。
    ///
    /// 一般不需要：推进/冻结直接在本值上做（[`abs_art::TrMockClock`] 已实现）。
    /// 需要时钟本身（例如传给 `LocalScope::block_on_advancing`）时用这个。
    pub fn clock(&self) -> &C {
        &self.clock_
    }

    /// 拆出被装饰的运行时值。
    pub fn into_inner(self) -> R {
        self.inner_
    }
}

impl<R, C: ManualClockApi> TrMockClock for ManualTime<R, C> {
    type Advance<'a>
        = MockAdvance<C>
    where
        Self: 'a;
    type AdvanceUntil<'a>
        = MockAdvance<C>
    where
        Self: 'a;

    fn pause(&self) {
        self.clock_.set_frozen(true);
    }

    fn resume(&self) {
        self.clock_.set_frozen(false);
    }

    fn is_paused(&self) -> bool {
        self.clock_.is_frozen()
    }

    fn advance(&self, by: Duration) -> Self::Advance<'_> {
        MockAdvance::by_(self.clock_.clone(), by)
    }

    fn advance_until(&self, at: Self::Instant) -> Self::AdvanceUntil<'_> {
        MockAdvance::until_(self.clock_.clone(), at)
    }
}

impl<R, C: ManualClockApi> TrDelay for ManualTime<R, C> {
    type Delay = C::Delay;

    fn delay(&self, duration: Duration) -> Self::Delay {
        self.clock_.sleep(duration)
    }
}

impl<R, C: ManualClockApi> TrClock for ManualTime<R, C> {
    type Instant = C::Instant;

    fn now(&self) -> Self::Instant {
        self.clock_.now()
    }
}

impl<R, C: ManualClockApi> TrTime for ManualTime<R, C> {
    type Interval = C::Interval;

    fn interval(&self, period: Duration) -> Self::Interval {
        self.clock_.interval(period)
    }
}

// impl<R: TrAsyncRuntime, C: ManualClockApi> TrAsyncRuntime for ManualTime<R, C> {
//     type JoinHandle<T> = R::JoinHandle<T> where T: 'static;
//
//     fn about(&self) -> RuntimeTag {
//         self.inner_.about()
//     }
// }

impl<R: TrSpawnSend, C: ManualClockApi> TrSpawnSend for ManualTime<R, C> {
    type JoinHandle<T>
        = R::JoinHandle<T>
    where
        T: 'static;

    fn spawn<F>(&self, future: F) -> Self::JoinHandle<<F as Future>::Output>
    where
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        self.inner_.spawn(future)
    }
}

impl<R: TrSpawnBlocking, C: ManualClockApi> TrSpawnBlocking for ManualTime<R, C> {
    type JoinHandle<T>
        = R::JoinHandle<T>
    where
        T: 'static;

    fn spawn_blocking<F, T>(&self, f: F) -> Self::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.inner_.spawn_blocking(f)
    }
}

impl<R: TrBlockOn, C: ManualClockApi> TrBlockOn for ManualTime<R, C> {
    fn block_on<F>(&self, f: F) -> <F as Future>::Output
    where
        F: Future,
    {
        self.inner_.block_on(f)
    }
}

impl<R: Clone, C: ManualClockApi> Clone for ManualTime<R, C> {
    /// 克隆装饰器：被装饰的值与手动时钟各克隆一份。
    ///
    /// [`ManualClock`] 的克隆**共享同一份时钟状态**（内部是 `Arc`），因此克隆出来的
    /// 两个装饰器读同一个时刻、走同一张到期表——与克隆各后端的 `Runtime` 语义一致。
    ///
    /// 之所以需要它：`ManualTime` 是「运行时值的装饰器」，而消费方常要求运行时值
    /// 可克隆（例如库侧把同一份运行时值分发给核心与若干循环）。各后端的 `Runtime`
    /// 都实现了 `Clone`，本类型因此也应当实现。
    fn clone(&self) -> Self {
        Self {
            inner_: self.inner_.clone(),
            clock_: self.clock_.clone(),
        }
    }
}

impl<R: fmt::Debug, C: ManualClockApi> fmt::Debug for ManualTime<R, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManualTime")
            .field("inner", &self.inner_)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    //! [`ManualTime`] 的覆盖与委托。

    use core::time::Duration;

    use abs_art::{RuntimeTag, TrAsyncRuntime, TrClock, TrDelay, TrInterval, TrTime};

    use super::ManualTime;
    use crate::{
        clock::{ManualClock, ManualClockApi},
        instant::MockInstant,
        support_::FakeRt_,
    };

    /// 目的：验证装饰器的时间来自手动时钟（而非 inner），身份则委托给 inner。
    ///
    /// 手段：构造 `ManualTime<FakeRt_>`，读 `about()`、推进时钟后读 `now()`。
    ///
    /// 判断：`about()` 为 `Smol`；`now()` 随手动时钟推进到 5s。
    #[test]
    fn time_comes_from_the_clock_identity_from_inner() {
        let clock = ManualClock::new();
        let value = ManualTime::new(FakeRt_, clock.clone());

        // assert_eq!(value.about(), RuntimeTag::Smol);
        clock.advance_by(Duration::from_secs(5));
        assert_eq!(value.now().as_millis(), 5_000);
        assert_eq!(value.clock().now().as_millis(), 5_000);
    }

    /// 目的：验证 `delay` 由手动时钟产出——未推进时 `Pending`，推进到点后 `Ready`。
    ///
    /// 手段：手工 poll `value.delay(100ms)`（`Waker::noop()`）。
    ///
    /// 判断：初始 `Pending`；`advance(100ms)` 后 `Ready`。
    #[test]
    fn delay_is_driven_by_the_manual_clock() {
        use core::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };

        let clock = ManualClock::new();
        let value = ManualTime::new(FakeRt_, clock.clone());
        let mut delay = pin!(value.delay(Duration::from_millis(100)));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert!(matches!(delay.as_mut().poll(&mut cx), Poll::Pending));
        clock.advance_by(Duration::from_millis(100));
        assert!(matches!(delay.as_mut().poll(&mut cx), Poll::Ready(())));
    }

    /// 目的：验证 `TrTime::interval` 也来自手动时钟，且首次 tick 立即完成。
    ///
    /// 手段：取周期 50ms 的 interval，手工 poll 第一个 tick。
    ///
    /// 判断：立即 `Ready`。
    #[test]
    fn interval_comes_from_the_manual_clock() {
        use core::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };

        let clock = ManualClock::new();
        let value = ManualTime::new(FakeRt_, clock.clone());
        let mut interval = value.interval(Duration::from_millis(50));
        let mut first = pin!(interval.tick());
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert!(matches!(first.as_mut().poll(&mut cx), Poll::Ready(())));
    }

    /// 目的：验证 `inner()` / `into_inner()` 取回被装饰的值。
    ///
    /// 手段：`ManualTime::new(FakeRt_, clock)` 后分别调用两者。
    ///
    /// 判断：都能拿到同一个 `FakeRt_`（其 `about()` 为 `Smol`）。
    #[test]
    fn inner_is_reachable() {
        let clock = ManualClock::new();
        let value = ManualTime::new(FakeRt_, clock.clone());
        assert_eq!(value.inner().about(), RuntimeTag::Smol);
        assert_eq!(value.into_inner().about(), RuntimeTag::Smol);
    }

    /// 目的：验证克隆出的装饰器与被克隆者**共享**同一个手动时钟。
    ///
    /// 手段：克隆 `ManualTime`，推进原值手上的那个 `ManualClock`，再读克隆体的 `now()`。
    ///
    /// 判断：克隆体读到推进后的时刻（5 s），原值同样——说明两者共享同一份时钟状态，
    /// 而不是各持一张独立的到期表；这也正是「克隆各后端 `Runtime`」的语义。
    #[test]
    fn clone_shares_the_same_manual_clock() {
        let clock = ManualClock::new();
        let value = ManualTime::new(FakeRt_, clock.clone());
        let cloned = value.clone();

        clock.advance_by(Duration::from_secs(5));

        assert_eq!(cloned.now().as_millis(), 5_000);
        assert_eq!(value.now().as_millis(), 5_000);
    }
}
