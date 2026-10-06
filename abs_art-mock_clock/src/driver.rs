//! 统一驱动：把测试主体包一层「Pending 时 tick 执行器、推进时钟、自唤醒」。

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use crate::clock::ManualClockApi;

/// 连续「既没跑到任务、也没推进时钟」的轮数上限；超过即判定为真死锁。
///
/// 取 64 而不是 1：没有 tick 钩子的后端（tokio）需要靠自己的 `block_on` 在两次
/// poll 之间推进被 spawn 的任务，可能要多轮才把定时器登记进来。
const STALL_LIMIT_: u32 = 64;

/// 「空闲即推进」的驱动适配器。
///
/// 把它交给后端的阻塞驱动入口（`block_on`）即可：
///
/// ```text
/// // smol
/// smol::block_on(Supervisor::new(body, clock.clone(), || executor.try_tick()))
/// // compio
/// runtime.block_on(Supervisor::new(body, clock.clone(), || runtime.run()))
/// // tokio（句柄没有 tick 钩子；LocalSet 承载 `!Send` 主体）
/// handle.block_on(local.run_until(Supervisor::new(body, clock.clone(), || false)))
/// ```
///
/// # 它做什么
///
/// 每次被 poll：先驱动主体；主体 `Pending` 时依次
///
/// 1. 调 `tick`（后端的执行器钩子：本轮有没有跑到任务）；
/// 2. 若时钟未冻结，推进到下一个到期时刻（唤醒到期任务）；
/// 3. 自唤醒，请求外层驱动循环立刻再进一次。
///
/// # Panics
///
/// 连续停滞达到内部上限（64 轮）「没跑到任务、也没有可推进的定时器」时 panic——
/// 把「忘了推进时钟 / 真的死锁 / 时钟被冻结却没人推」这类静默挂起变成响亮失败。
pub struct Supervisor<F, C: ManualClockApi, T: Fn() -> bool> {
    /// 测试主体。
    body_: Pin<Box<F>>,
    /// 手动时钟。
    clock_: C,
    /// 后端的执行器 tick 钩子（返回「本轮有没有跑到任务」）。
    tick_: Box<T>,
    /// 连续停滞轮数。
    stalled_: u32,
}

impl<F, C: ManualClockApi, T: Fn() -> bool> Supervisor<F, C, T> {
    /// 用主体、时钟与 tick 钩子构造驱动适配器。
    pub fn new(body: F, clock: C, tick: T) -> Self {
        Self {
            body_: Box::pin(body),
            clock_: clock,
            tick_: Box::new(tick),
            stalled_: 0,
        }
    }

    /// 本驱动使用的手动时钟。
    pub fn clock(&self) -> &C {
        &self.clock_
    }
}

impl<F: Future, C: ManualClockApi, T: Fn() -> bool> Future for Supervisor<F, C, T> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // 所有字段都是 `Unpin`（`Pin<Box<_>>` 与 `Box<_>` 恒为 `Unpin`，时钟 trait 要求
        // `Unpin`），因此 `Supervisor` 自动 `Unpin`，这里无需 unsafe 投影。
        let this = self.get_mut();

        if let Poll::Ready(value) = this.body_.as_mut().poll(cx) {
            return Poll::Ready(value);
        }

        let ran = (this.tick_)();
        let advanced = if this.clock_.is_frozen() {
            false
        } else {
            this.clock_.try_advance_to_next()
        };

        if ran || advanced {
            this.stalled_ = 0;
        } else {
            this.stalled_ = this.stalled_.saturating_add(1);
            assert!(
                this.stalled_ < STALL_LIMIT_,
                "手动时钟驱动停滞：既没有就绪任务、也没有可推进的定时器。\
                 常见原因：忘了 advance、时钟被 set_paused(true) 冻结、或主体真的在等一个\
                 永远不会到来的事件。"
            );
        }

        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    //! [`Supervisor`] 的推进与死锁判定。

    use alloc::string::{String, ToString};
    use core::future::pending;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};
    use core::time::Duration;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use abs_art::{TrClock, TrDelay};

    use super::Supervisor;
    use crate::clock::{ManualClock, ManualClockApi};
    use crate::decorator::ManualTime;
    use crate::instant::MockInstant;
    use crate::support_::FakeRt_;

    /// 目的：验证驱动会把主体推进到 delay 到期（无需任何真实运行时）。
    ///
    /// 手段：手工 poll `Supervisor`（`Waker::noop()`），主体是 `ManualTime::delay(1s)`。
    ///
    /// 判断：有限轮内 `Ready`，且时钟停在 1000ms。
    #[test]
    fn supervisor_advances_until_the_body_finishes() {
        let clock = ManualClock::new();
        let value = ManualTime::new(FakeRt_, clock.clone());
        let body_clock = clock.clone();
        let mut supervisor = pin!(Supervisor::new(
            async move {
                value.delay(Duration::from_millis(1_000)).await;
                body_clock.now().as_millis()
            },
            clock.clone(),
            || false,
        ));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        let mut out = None;
        for _ in 0..64 {
            if let Poll::Ready(value) = supervisor.as_mut().poll(&mut cx) {
                out = Some(value);
                break;
            }
        }

        assert_eq!(out, Some(1_000));
        assert_eq!(clock.now().as_millis(), 1_000);
    }

    /// 目的：验证测试主体可以自己 `advance(..).await` 推动时间（tokio 风格用法）。
    ///
    /// 手段：主体先 `advance(60s).await`，再 `delay(60s).await`，随后读时刻；
    /// 驱动负责把后面那 60 秒补上。
    ///
    /// 判断：取回 120_000ms。
    #[test]
    fn body_can_advance_the_clock_itself() {
        use abs_art::TrMockClock;

        let clock = ManualClock::new();
        let value = ManualTime::new(FakeRt_, clock.clone());
        let body_clock = clock.clone();
        let mut supervisor = pin!(Supervisor::new(
            async move {
                value.advance(Duration::from_secs(60)).await;
                value.delay(Duration::from_secs(60)).await;
                body_clock.now().as_millis()
            },
            clock.clone(),
            || false,
        ));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        let mut out = None;
        for _ in 0..128 {
            if let Poll::Ready(value) = supervisor.as_mut().poll(&mut cx) {
                out = Some(value);
                break;
            }
        }

        assert_eq!(out, Some(120_000));
    }

    /// 目的：验证「没有定时器可推进」时驱动会响亮 panic，而不是静默挂起。
    ///
    /// 手段：主体是 `pending()`，连续 poll 超过停滞上限。
    ///
    /// 判断：发生 panic 且消息含「停滞」。
    #[test]
    fn supervisor_panics_on_real_deadlock() {
        let clock = ManualClock::new();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut supervisor = pin!(Supervisor::new(pending::<()>(), clock.clone(), || false));
            let waker = Waker::noop();
            let mut cx = Context::from_waker(waker);
            for _ in 0..200 {
                let _ = supervisor.as_mut().poll(&mut cx);
            }
        }));
        let payload = outcome.expect_err("应当 panic");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        assert!(message.contains("停滞"), "实际消息：{message}");
    }

    /// 目的：验证时钟被冻结时驱动**不**自动推进（冻结即「只有显式 `advance` 才动」）。
    ///
    /// 手段：`set_paused(true)` 后驱动一个等 1s 的主体。
    ///
    /// 判断：发生停滞 panic。
    #[test]
    fn paused_clock_is_not_advanced_by_the_driver() {
        let clock = ManualClock::new();
        clock.set_frozen(true);
        let value = ManualTime::new(FakeRt_, clock.clone());
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut supervisor = pin!(Supervisor::new(
                async {
                    value.delay(Duration::from_millis(1_000)).await;
                },
                clock.clone(),
                || false,
            ));
            let waker = Waker::noop();
            let mut cx = Context::from_waker(waker);
            for _ in 0..200 {
                let _ = supervisor.as_mut().poll(&mut cx);
            }
        }));
        assert!(outcome.is_err(), "冻结的时钟不该被驱动推进");
    }
}
