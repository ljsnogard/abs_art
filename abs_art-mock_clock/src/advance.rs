//! 手动时钟的推进 future。

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;

use crate::clock::ManualClockApi;

/// [`abs_art::TrMockClock::advance`] 与 [`abs_art::TrMockClock::advance_until`] 的返回类型。
///
/// # 为什么是具名类型（而不是 `impl Future`）
///
/// `abs_art::TrMockClock` 用 GAT 而不是 RPITIT 声明这两个返回类型，正是为了让 auto
/// trait 对调用方可见。本类型因此把「是否 `Send`」摊开：它只在 `C: Send` 时是 `Send`
/// ——泛型代码里的 `Send` 判定不再依赖不透明类型。
///
/// # 语义
///
/// 首次 poll 时推进时钟（`advance` 推进 `by`，`advance_until` 推进到绝对时刻），随后
/// 立即完成。与真实运行时一致：**推进发生在被 poll 的那一刻**，而不是构造 future 时。
pub struct MockAdvance<C: ManualClockApi> {
    /// 时钟句柄。
    clock_: C,
    /// 「推进一段」的目标（与 `until_` 二选一）。
    by_: Option<Duration>,
    /// 「推进到绝对时刻」的目标（与 `by_` 二选一）。
    ///
    /// 装在 `Box` 里是为了让本类型**无条件 `Unpin`**：`TrClock::Instant` 只保证
    /// `Copy + Ord + Add + Sub + 'static`，并不保证 `Unpin`，而 `Pin::get_mut`
    /// 需要 `Self: Unpin`。`Box<T>: Unpin` 无条件成立（一次分配换掉一层约束）。
    until_: Option<Box<C::Instant>>,
    /// 是否已经推进过（再次 poll 直接完成）。
    done_: bool,
}

impl<C: ManualClockApi> MockAdvance<C> {
    /// 「推进 `by`」的 future（由 [`abs_art::TrMockClock::advance`] 调用）。
    pub(crate) fn by_(clock: C, by: Duration) -> Self {
        Self {
            clock_: clock,
            by_: Some(by),
            until_: None,
            done_: false,
        }
    }

    /// 「推进到 `at`」的 future（由 [`abs_art::TrMockClock::advance_until`] 调用）。
    pub(crate) fn until_(clock: C, at: C::Instant) -> Self {
        Self {
            clock_: clock,
            by_: None,
            until_: Some(Box::new(at)),
            done_: false,
        }
    }
}

impl<C: ManualClockApi> Future for MockAdvance<C> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        // 所有字段都是 `Unpin`（时钟 trait 要求 `Unpin`），无需 unsafe 投影。
        let this = self.get_mut();
        if !this.done_ {
            this.done_ = true;
            if let Some(at) = this.until_.as_ref() {
                this.clock_.advance_to(**at);
            } else if let Some(by) = this.by_ {
                this.clock_.advance_by(by);
            }
        }
        Poll::Ready(())
    }
}

#[cfg(test)]
mod tests {
    //! [`MockAdvance`] 的行为。

    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};
    use core::time::Duration;

    use abs_art::{TrClock, TrMockClock};

    use crate::clock::ManualClock;
    use crate::instant::MockInstant;

    /// 目的：验证推进发生在**被 poll** 时，而不是构造 future 时。
    ///
    /// 手段：构造 `clock.advance(7s)` 后先读时刻，再 poll 一次，再读时刻。
    ///
    /// 判断：poll 前为 0ms；poll 返回 `Ready` 后为 7000ms。
    #[test]
    fn advance_happens_on_first_poll() {
        let clock = ManualClock::new();
        let mut advance = pin!(clock.advance(Duration::from_secs(7)));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert_eq!(clock.now().as_millis(), 0);
        assert!(matches!(advance.as_mut().poll(&mut cx), Poll::Ready(())));
        assert_eq!(clock.now().as_millis(), 7_000);
    }

    /// 目的：验证 `advance_until` 推进到绝对时刻，且不会把时钟往回拨。
    ///
    /// 手段：先同步推进到 10s，再 `advance_until(4s)` 与 `advance_until(30s)` 各 poll 一次。
    ///
    /// 判断：分别停在 10_000ms 与 30_000ms。
    #[test]
    fn advance_until_is_monotonic() {
        use crate::clock::ManualClockApi;

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
}
