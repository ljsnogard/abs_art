//! 统一驱动：把测试主体包一层「Pending 时先 tick 执行器；执行器没活才推进时钟；再自唤醒」。

use alloc::boxed::Box;
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use crate::clock::ManualClockApi;

/// 连续「既没跑到任务、也没推进时钟」的轮数上限；超过即判定为真死锁。
const STALL_LIMIT_: u32 = 64;

/// 「执行器没活才推进」的驱动适配器。
///
/// 把它交给后端的阻塞驱动入口（`block_on`）即可：
///
/// ```text
/// // smol（本地队列有同步 tick）
/// smol::block_on(Supervisor::new(body, clock.clone(), || executor.try_tick()))
/// // compio（`Runtime::run` 返回「队列里还有任务吗」）
/// runtime.block_on(Supervisor::new(body, clock.clone(), || runtime.run()))
/// // tokio（`LocalSet::tick` 是 crate 私有的；由本后端的「唤醒登记」折算出同义信号）
/// handle.block_on(local.run_until(Supervisor::new(body, clock.clone(), || woke.take())))
/// ```
///
/// # 它做什么
///
/// 每次被 poll：先驱动主体；主体 `Pending` 时依次
///
/// 1. 调 `tick`（后端的执行器钩子：**执行器这一轮还有活吗**）；
/// 2. **连续两轮都没有活、且时钟未冻结**时，才推进到下一个到期时刻（唤醒到期任务）；
/// 3. 自唤醒，请求外层驱动循环立刻再进一次。
///
/// 第 1、2 步的 `ran` 判据不可省：执行器还有就绪任务时推进，等于把本该由任务链一步
/// 一步走完的事件顺序，压缩进「一个定时器周期」的虚拟时间里——凡是拿 `now` 做
/// 判据的逻辑（空闲超时、保活、退避）都会在握手 / 事件链走完之前误触发。
///
/// **为什么是「连续两轮」而不是「一轮」**：唤醒是链式的，后端一次 tick 跑完当前
/// 就绪任务后队列可能正好为空，而它刚刚唤醒的下一环还没被驱动。要求连续两轮都报
/// 「没活」，才能把「链条真的走完了」与「链条正卡在两环之间」区分开。这样一来，
/// 后端只需要回答「有没有活」，不必回答「唤醒链是否已经走完」——后者在 tokio /
/// compio 的公开 API 上都拿不到。
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
    /// 连续「执行器没活」的轮数。
    idle_streak_: u32,
}

impl<F, C: ManualClockApi, T: Fn() -> bool> Supervisor<F, C, T> {
    /// 用主体、时钟与 tick 钩子构造驱动适配器。
    pub fn new(body: F, clock: C, tick: T) -> Self {
        Self {
            body_: Box::pin(body),
            clock_: clock,
            tick_: Box::new(tick),
            stalled_: 0,
            idle_streak_: 0,
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

        // 「执行器还有活吗」由后端钩子回答。**有活就不推进**：此刻该做的是把执行器
        // 继续跑下去，而不是把虚拟时间往前拨一个定时器周期。
        let ran = (this.tick_)();
        if ran {
            this.idle_streak_ = 0;
        } else {
            this.idle_streak_ = this.idle_streak_.saturating_add(1);
        }
        // 「连续两轮没活」才推进：单单一轮没活，可能只是「刚跑完最后一个任务、它唤醒
        // 的下一环还没被驱动」。这样后端只需要报告「有没有活」，不必报告「唤醒链是否
        // 已经走完」。
        let advanced = if ran || this.clock_.is_frozen() || this.idle_streak_ < 2 {
            false
        } else {
            let did = this.clock_.try_advance_to_next();
            if did {
                this.idle_streak_ = 0;
            }
            did
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
    use core::{
        future::pending,
        pin::pin,
        task::{Context, Poll, Waker},
        time::Duration,
    };
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use abs_art::{TrClock, TrDelay};

    use super::Supervisor;
    use crate::{
        clock::{ManualClock, ManualClockApi},
        decorator::ManualTime,
        instant::MockInstant,
        support_::FakeRt_,
    };

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
