//! `Supervisor` 的**推进判据契约**：虚拟时间只应在「执行器确实没有别的事可干」时前进。
//!
//! 背景与完整证据链见 `smux_v1/dev-notes/timer-mock-clock-and-generic-drop-20261006-1625.md`
//! §11（跨三端最小复现）与 §12（修法验证）。
//!
//! 两条用例是一对对照：`tick` 报「执行器有活」时**不得**推进虚拟时间；报「没活」
//! 时才推进。前者钉住「事件链没走完就不许拨表」，后者防「改成永不推进」的新死结。

use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use abs_art::TrClock;
use abs_art_mock_clock::{ManualClock, ManualClockApi, Supervisor};

/// 让出一次执行权：自唤醒一次后返回 `Pending`。
async fn yield_once_() {
    let mut first = true;
    core::future::poll_fn(move |_cx| {
        if first {
            first = false;
            core::task::Poll::Pending
        } else {
            core::task::Poll::Ready(())
        }
    })
    .await;
}

/// 一个**忙轮询**的最小执行器：反复 poll 直到就绪。
///
/// 本 crate 没有异步运行时依赖，而 `Supervisor` 与被测主体都会自唤醒，因此
/// 「忽略 waker、循环 poll」就足以驱动它们——这也让本用例不掺入任何执行器语义。
fn block_on_busy_<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        if let Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
    }
}

/// 在 `tick` 恒为 `tick_value` 的前提下，自旋 4 轮、每轮登记一个 `now + 500ms`
/// 的定时器（保证时钟总有下一个到期时刻），返回结束时的虚拟时刻。
fn spin_with_tick_(tick_value: bool) -> u64 {
    let clock = ManualClock::new();
    let clock_in_body = clock.clone();
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);

    block_on_busy_(Supervisor::new(
        async move {
            for _ in 0..4 {
                let mut delay = pin!(clock_in_body.sleep(Duration::from_millis(500u64)));
                let _ = Future::poll(delay.as_mut(), &mut cx);
                yield_once_().await;
            }
            clock_in_body.now().get()
        },
        clock.clone(),
        move || tick_value,
    ))
}

/// 测试目标：**执行器仍「有活」（tick 报 `true`）时不得推进虚拟时钟**。
///
/// - 手段：tick 钩子恒返回 `true`，主体只做「登记定时器 + 让出」共 4 轮。
/// - 判断：结束时的虚拟时刻必须是 `0`——纯调度让出不构成「可以推进时间」的理由。
///
#[test]
fn advance_must_wait_until_executor_is_idle_() {
    let out = spin_with_tick_(true);
    assert_eq!(
        out, 0u64,
        "执行器仍有活（tick=true）时不该消耗虚拟时间，实际推进到 {out} ms"
    );
}

/// 测试目标：**执行器没活时，推进照常发生**（与上一条构成对照，防「永不推进」）。
///
/// - 手段：同一场景，tick 恒返回 `false`（= 执行器没有就绪工作）。
/// - 判断：`Supervisor` 要求**连续两轮**「没活」才推进一格（单轮没活可能只是「刚跑完
///   最后一个任务、它唤醒的下一环还没被驱动」）；每轮登记 `now + 500ms`，因此 4 轮
///   让出推进 2 格，虚拟时刻为 `1000 ms`。
#[test]
fn advance_happens_when_executor_is_idle_() {
    let out = spin_with_tick_(false);
    assert_eq!(
        out, 1000u64,
        "执行器连续没活（tick=false）时应当推进，实际为 {out} ms"
    );
}
