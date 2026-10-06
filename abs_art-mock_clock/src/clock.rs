//! 手动时钟：共享的时刻状态 + 到期唤醒表。

use alloc::{sync::Arc, vec::Vec};
use core::{fmt, future::Future, marker::PhantomData, task::Waker, time::Duration};

use abs_art::{TrClock, TrInterval, TrMockClock};
use atomic_sync::mutex::preemptive::SpinningMutexOwned;

use crate::{
    advance::MockAdvance,
    delay::{MockDelay, MockInterval},
    instant::{MillisInstant, MockInstant},
};

/// 手动时钟的**机制**接口——扩展点。
///
/// 它**继承** [`abs_art::TrMockClock`]（后者又继承 [`abs_art::TrClock`]）：也就是说，
/// 「能力」在 `abs_art` 里（一个可手动推进的时钟），而这里只补上**实现一份手动时钟
/// 所需的机制**（delay/interval 的构造、到期表推进的内部原语）。
///
/// 内置实现是 [`ManualClock`]；需要别的共享状态（例如 no_std 的自旋锁）时实现本 trait
/// 即可接入 [`crate::ManualTime`] 与 [`crate::Supervisor`]。
///
/// # 为什么时间原语分成「能力」与「机制」两层
///
/// | 层 | 在哪 | 谁用 |
/// | --- | --- | --- |
/// | [`abs_art::TrClock`] / [`abs_art::TrMockClock`] | `abs_art` | 业务代码 / 测试代码 |
/// | 本 trait（`sleep` / `interval` / `try_advance_to_next` …） | 本 crate | [`crate::ManualTime`] 与 [`crate::Supervisor`] |
///
/// # 契约
///
/// 1. [`advance_by`](Self::advance_by) / [`advance_to`](Self::advance_to) /
///    [`try_advance_to_next`](Self::try_advance_to_next) 必须**唤醒**期间到期的 waker，
///    且唤醒动作应在**释放内部锁之后**进行（避免重入死锁）；
/// 2. [`try_advance_to_next`](Self::try_advance_to_next) 在没有「尚未到期的定时器」时
///    返回 `false`——[`crate::Supervisor`] 依赖这个信号判定真死锁；
/// 3. [`is_frozen`](Self::is_frozen) 为 `true` 时，[`crate::Supervisor`] 不自动推进。
pub trait ManualClockApi: abs_art::TrMockClock + Clone + Unpin + 'static {
    /// 本时钟产出的 delay future（即 `TrDelay::Delay`）。
    type Delay: Future<Output = ()>;

    /// 本时钟产出的周期源（即 `TrTime::Interval`）。
    type Interval: TrInterval;

    /// 下一个「尚未到期」的截止时刻。
    fn next_deadline(&self) -> Option<Self::Instant>;

    /// 推进到下一个到期时刻并唤醒到期任务；没有可推进的定时器时返回 `false`。
    ///
    /// 这是**驱动内部**用的同步原语（[`crate::Supervisor`] 在 poll 里调它）。
    fn try_advance_to_next(&self) -> bool;

    /// 同步推进 `by`（[`abs_art::TrMockClock::advance`] 的底层原语）。
    fn advance_by(&self, by: Duration);

    /// 同步推进到 `at`（[`abs_art::TrMockClock::advance_until`] 的底层原语）。
    fn advance_to(&self, at: Self::Instant);

    /// 是否处于「冻结」状态（冻结时驱动不自动推进）。
    fn is_frozen(&self) -> bool;

    /// 设置冻结状态。
    fn set_frozen(&self, frozen: bool);

    /// 造一个睡 `duration` 的 delay future。
    fn sleep(&self, duration: Duration) -> Self::Delay;

    /// 造一个周期为 `period` 的周期源（首次 tick 立即完成，与各后端契约一致）。
    ///
    /// # Panics
    ///
    /// `period` 为零时 panic（与三个后端的 `TrTime::interval` 契约一致）。
    fn interval(&self, period: Duration) -> Self::Interval;
}

/// 手动时钟的内部状态。
struct Inner_<I: MockInstant> {
    /// 当前时刻（毫秒刻度）。
    now_millis: u64,
    /// 冻结标志：为真时驱动不自动推进。
    frozen: bool,
    /// 到期唤醒表：`(截止毫秒, waker)`。
    timers: Vec<(u64, Waker)>,
    /// 只是把时刻类型带进类型参数里（`fn() -> I` 保证不引入 Send/Sync 约束）。
    marker_: PhantomData<fn() -> I>,
}

/// 手动时钟：毫秒刻度，`Arc<Mutex<_>>` 共享，可跨线程使用。
///
/// 时刻类型默认为 [`MillisInstant`]，也可以换成自己实现的 [`MockInstant`]。
///
/// # Examples
///
/// ```
/// use core::time::Duration;
/// use abs_art::TrClock;
/// use abs_art_mock_clock::{ManualClock, ManualClockApi, MockInstant};
///
/// let clock = ManualClock::new();
/// clock.advance_by(Duration::from_secs(3));
/// assert_eq!(clock.now().as_millis(), 3_000);
/// ```
pub struct ManualClock<I: MockInstant = MillisInstant> {
    inner_: Arc<SpinningMutexOwned<Inner_<I>>>,
}

impl<I: MockInstant> Clone for ManualClock<I> {
    fn clone(&self) -> Self {
        Self {
            inner_: Arc::clone(&self.inner_),
        }
    }
}

impl<I: MockInstant> Default for ManualClock<I> {
    fn default() -> Self {
        Self::with_instant()
    }
}

impl<I: MockInstant> fmt::Debug for ManualClock<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with_(|inner| {
            f.debug_struct("ManualClock")
                .field("now_millis", &inner.now_millis)
                .field("frozen", &inner.frozen)
                .field("timers", &inner.timers.len())
                .finish()
        })
    }
}

impl ManualClock<MillisInstant> {
    /// 造一个毫秒刻度的手动时钟（最常用）。
    ///
    /// 刻意**只**实现在默认时刻类型上：写 `ManualClock::new()` 时不需要任何类型
    /// 标注（泛型构造在表达式位置会要求标注，这个家族已经吃过一次这个亏）。
    #[must_use]
    pub fn new() -> Self {
        Self::with_instant()
    }
}

impl<I: MockInstant> ManualClock<I> {
    /// 造一个指定时刻类型的手动时钟。
    #[must_use]
    pub fn with_instant() -> Self {
        Self {
            inner_: Arc::new(SpinningMutexOwned::new_owned(Inner_ {
                now_millis: 0,
                frozen: false,
                timers: Vec::new(),
                marker_: PhantomData,
            })),
        }
    }

    /// 在自旋锁保护的临界区里跑 `f`。
    ///
    /// 锁来自 `atomic_sync` 的 [`SpinningMutexOwned`]（`no_std`），而不是自造的锁：
    /// 临界区只做内存操作（读/写时刻、推入/摘出 waker），因此自旋是合适的。
    ///
    /// 用闭包而不是返回守卫：`atomic_sync` 的守卫借用 `LockSession`，无法从本函数里
    /// 逃逸出去；顺带保证**唤醒 waker 一定发生在锁外**（调用方在 `with_` 返回后再唤醒）。
    fn with_<R>(&self, f: impl FnOnce(&mut Inner_<I>) -> R) -> R {
        let mut session = self.inner_.lock_session();
        // `wait()` 内部使用不可取消的令牌，`Err` 理论上不可达；这里用循环把 `Result`
        // 收敛掉——既不在库代码里 `unwrap`/`expect`，也不会 panic。
        let mut guard = loop {
            match session.lock().wait() {
                Ok(guard) => break guard,
                Err(_) => continue,
            }
        };
        f(&mut guard)
    }

    /// 当前时刻的毫秒刻度（内部用）。
    pub(crate) fn now_millis_(&self) -> u64 {
        self.with_(|inner| inner.now_millis)
    }

    /// 登记一个到期 waker（内部用）。
    pub(crate) fn register_(&self, deadline_millis: u64, waker: Waker) {
        self.with_(|inner| inner.timers.push((deadline_millis, waker)));
    }

    /// 推进到下一个到期时刻并**在锁外**唤醒到期任务。
    fn jump_to_next_(&self) -> bool {
        let due = self.with_(|inner| {
            let now = inner.now_millis;
            let next = inner
                .timers
                .iter()
                .map(|(deadline, _)| *deadline)
                .filter(|deadline| *deadline > now)
                .min()?;
            inner.now_millis = next;
            Some(take_due_(&mut inner.timers, next))
        });
        let Some(due) = due else {
            return false;
        };
        for waker in due {
            waker.wake();
        }
        true
    }

    /// 推进 `by` 毫秒并**在锁外**唤醒到期任务。
    fn jump_by_(&self, by_millis: u64) {
        let due = self.with_(|inner| {
            inner.now_millis = inner.now_millis.saturating_add(by_millis);
            let now = inner.now_millis;
            take_due_(&mut inner.timers, now)
        });
        for waker in due {
            waker.wake();
        }
    }
}

/// 从唤醒表里摘掉所有 `deadline <= now` 的条目并返回它们的 waker。
fn take_due_(timers: &mut Vec<(u64, Waker)>, now_millis: u64) -> Vec<Waker> {
    let mut due = Vec::new();
    let mut index = 0;
    while index < timers.len() {
        if timers[index].0 <= now_millis {
            let (_, waker) = timers.swap_remove(index);
            due.push(waker);
        } else {
            index += 1;
        }
    }
    due
}

impl<I: MockInstant> TrClock for ManualClock<I> {
    type Instant = I;

    fn now(&self) -> Self::Instant {
        I::from_millis(self.now_millis_())
    }
}

impl<I: MockInstant> TrMockClock for ManualClock<I> {
    type Advance<'a>
        = MockAdvance<ManualClock<I>>
    where
        Self: 'a;
    type AdvanceUntil<'a>
        = MockAdvance<ManualClock<I>>
    where
        Self: 'a;

    fn pause(&self) {
        self.set_frozen(true);
    }

    fn resume(&self) {
        self.set_frozen(false);
    }

    fn is_paused(&self) -> bool {
        self.is_frozen()
    }

    fn advance(&self, by: Duration) -> Self::Advance<'_> {
        MockAdvance::by_(self.clone(), by)
    }

    fn advance_until(&self, at: Self::Instant) -> Self::AdvanceUntil<'_> {
        MockAdvance::until_(self.clone(), at)
    }
}

impl<I: MockInstant> ManualClockApi for ManualClock<I> {
    type Delay = MockDelay<I>;
    type Interval = MockInterval<I>;

    fn next_deadline(&self) -> Option<Self::Instant> {
        let now = self.now_millis_();
        let deadline = self.with_(|inner| {
            inner
                .timers
                .iter()
                .map(|(deadline, _)| *deadline)
                .filter(|deadline| *deadline > now)
                .min()
        })?;
        Some(I::from_millis(deadline))
    }

    fn try_advance_to_next(&self) -> bool {
        self.jump_to_next_()
    }

    fn advance_by(&self, by: Duration) {
        self.jump_by_(by.as_millis() as u64);
    }

    fn advance_to(&self, at: Self::Instant) {
        let target = at.as_millis();
        let now = self.now_millis_();
        self.jump_by_(target.saturating_sub(now));
    }

    fn is_frozen(&self) -> bool {
        self.with_(|inner| inner.frozen)
    }

    fn set_frozen(&self, frozen: bool) {
        self.with_(|inner| inner.frozen = frozen);
    }

    fn sleep(&self, duration: Duration) -> Self::Delay {
        let deadline = self
            .now_millis_()
            .saturating_add(duration.as_millis() as u64);
        MockDelay::new_(self.clone(), deadline)
    }

    fn interval(&self, period: Duration) -> Self::Interval {
        assert!(period > Duration::ZERO, "`period` must be non-zero.");
        MockInterval::new_(self.clone(), period.as_millis() as u64)
    }
}

#[cfg(test)]
mod tests {
    //! [`ManualClock`] 的推进与唤醒语义。

    use alloc::task::Wake;
    use core::time::Duration;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use abs_art::TrClock;

    use super::{ManualClock, ManualClockApi};
    use crate::instant::MockInstant;

    /// 只数唤醒次数的 waker。
    struct Counter_(Arc<AtomicUsize>);

    impl Wake for Counter_ {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 目的：验证 `advance_to_next` 推进到下一个到期时刻，并且**只在**到期后唤醒。
    ///
    /// 手段：登记两个 waker（100ms 与 200ms），分两次推进。
    ///
    /// 判断：第一次推进后仅 100ms 的 waker 被唤醒、`now()` 为 100ms；第二次到 200ms。
    #[test]
    fn advance_to_next_wakes_in_deadline_order() {
        let clock = ManualClock::new();
        let low = Arc::new(AtomicUsize::new(0));
        let high = Arc::new(AtomicUsize::new(0));
        clock.register_(100, Arc::new(Counter_(Arc::clone(&low))).into());
        clock.register_(200, Arc::new(Counter_(Arc::clone(&high))).into());

        assert!(clock.try_advance_to_next());
        assert_eq!(clock.now().as_millis(), 100);
        assert_eq!(low.load(Ordering::SeqCst), 1);
        assert_eq!(high.load(Ordering::SeqCst), 0);

        assert!(clock.try_advance_to_next());
        assert_eq!(clock.now().as_millis(), 200);
        assert_eq!(high.load(Ordering::SeqCst), 1);
    }

    /// 目的：验证没有「尚未到期的定时器」时 `advance_to_next` 返回 `false`。
    ///
    /// 手段：空时钟调一次；再登记一个已经到期的 waker 后调一次。
    ///
    /// 判断：两次都为 `false`（已到期的条目不构成「可推进」）。
    #[test]
    fn advance_to_next_reports_nothing_to_do() {
        let clock = ManualClock::new();
        assert!(!clock.try_advance_to_next());

        let counter = Arc::new(AtomicUsize::new(0));
        clock.register_(0, Arc::new(Counter_(counter)).into());
        assert!(!clock.try_advance_to_next());
    }

    /// 目的：验证 `advance_by` 无条件推进并唤醒期间到期的任务。
    ///
    /// 手段：登记 100ms 的 waker，一次推进 250ms。
    ///
    /// 判断：`now()` 为 250ms 且该 waker 被唤醒一次。
    #[test]
    fn advance_by_duration_wakes_overdue() {
        let clock = ManualClock::new();
        let counter = Arc::new(AtomicUsize::new(0));
        clock.register_(100, Arc::new(Counter_(Arc::clone(&counter))).into());

        clock.advance_by(Duration::from_millis(250));

        assert_eq!(clock.now().as_millis(), 250);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    /// 目的：验证冻结标志可读可写（驱动是否自动推进由它决定）。
    ///
    /// 手段：读取初值、置真、再读。
    ///
    /// 判断：初值为 `false`，置真后为 `true`。
    #[test]
    fn pause_flag_round_trips() {
        let clock = ManualClock::new();
        assert!(!clock.is_frozen());
        clock.set_frozen(true);
        assert!(clock.is_frozen());
    }

    /// 目的：验证 `TrMockClock::advance` 是 future，且在**被 poll 时**才推进时刻。
    ///
    /// 手段：手工 poll `advance(7s)`（`Waker::noop()`），poll 前后各读一次时刻。
    ///
    /// 判断：poll 前为 0ms，poll 返回 `Ready` 后为 7000ms。
    #[test]
    fn async_advance_advances_when_polled() {
        use core::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };

        use abs_art::TrMockClock;

        let clock = ManualClock::new();
        let mut advance = pin!(clock.advance(Duration::from_secs(7)));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert_eq!(clock.now().as_millis(), 0);
        assert!(matches!(advance.as_mut().poll(&mut cx), Poll::Ready(())));
        assert_eq!(clock.now().as_millis(), 7_000);
    }

    /// 目的：验证 `advance_until` 推进到指定时刻、且不会把时钟往回拨。
    ///
    /// 手段：先 `advance_by(10s)`，再 `advance_until(4s)` 与 `advance_until(30s)`。
    ///
    /// 判断：分别停在 10s 与 30s。
    #[test]
    fn advance_until_never_goes_backwards() {
        use core::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };

        use abs_art::TrMockClock;

        let clock = ManualClock::new();
        clock.advance_by(Duration::from_secs(10));
        let waker = Waker::noop();

        {
            let mut back = pin!(clock.advance_until(crate::instant::MillisInstant::new(4_000)));
            let mut cx = Context::from_waker(waker);
            assert!(matches!(back.as_mut().poll(&mut cx), Poll::Ready(())));
        }
        assert_eq!(clock.now().as_millis(), 10_000);

        let mut forward = pin!(clock.advance_until(crate::instant::MillisInstant::new(30_000)));
        let mut cx = Context::from_waker(waker);
        assert!(matches!(forward.as_mut().poll(&mut cx), Poll::Ready(())));
        assert_eq!(clock.now().as_millis(), 30_000);
    }

    /// 目的：验证时钟在多线程下互斥正确——并发推进不丢更新（锁由 `atomic_sync` 提供）。
    ///
    /// 手段：4 条线程各调 `advance_by(1ms)` 共 5_000 次。
    ///
    /// 判断：最终时刻恰为 20_000ms；若临界区没被正确保护，读-改-写会互相覆盖而使结果变小。
    #[test]
    fn concurrent_advance_does_not_lose_updates() {
        use std::{sync::Arc, thread};

        use crate::clock::ManualClockApi;

        let clock = Arc::new(ManualClock::new());
        let handles: std::vec::Vec<_> = (0..4)
            .map(|_| {
                let clock = Arc::clone(&clock);
                thread::spawn(move || {
                    for _ in 0..5_000 {
                        clock.advance_by(Duration::from_millis(1));
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("线程不应 panic");
        }

        assert_eq!(clock.now().as_millis(), 20_000);
    }

    /// 目的：验证 `next_deadline` 只报「尚未到期」的最小截止时刻。
    ///
    /// 手段：登记 300ms 与 100ms 两个 waker，读 `next_deadline`。
    ///
    /// 判断：返回 100ms。
    #[test]
    fn next_deadline_is_the_earliest_future_one() {
        let clock = ManualClock::new();
        assert!(clock.next_deadline().is_none());

        clock.register_(
            300,
            Arc::new(Counter_(Arc::new(AtomicUsize::new(0)))).into(),
        );
        clock.register_(
            100,
            Arc::new(Counter_(Arc::new(AtomicUsize::new(0)))).into(),
        );

        assert_eq!(clock.next_deadline().map(|i| i.as_millis()), Some(100));
    }
}
