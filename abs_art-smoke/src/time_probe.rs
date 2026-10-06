//! 跨后端 [`TrTime`] 行为契约的测试体（**泛型于 `R: TrTime`，收运行时值 `&R`**）。
//!
//! 与 `spawn_local` 那组同形：测试体只有**一份**，三个后端各拿它跑一遍，于是
//! 「同一份 trait、三个实现」最容易出的**语义漂移**变成可执行的判定。
//!
//! # 值化之后的形状变化（只改调用形状，不改判定）
//!
//! v0.3 的这些探针泛型于**类型** `T: TrTime`，调用点写 `<T as TrTime>::interval(p)`、
//! `<T as TrDelay>::delay(d)`——计时能力挂在类型上。v0.4 把能力挂到运行时**值**上，
//! 因此这里是 `rt.interval(p)` / `rt.delay(d)` / `rt.now()`，且多了一条类型级参数
//! 无法表达的新契约：[`probe_clock_is_monotonic_and_shares_delay_source`]。
//!
//! # 判定为什么用宽松上界
//!
//! 这里的用例要抓的是**结构性**偏差——「首次 tick 被推迟了一个周期」「锚定变成了
//! 每响之后再等一个周期」——而不是调度抖动。因此阈值一律留几百毫秒富余，宁可放过
//! 一次抖动，也不要变成一条随机红的用例。
//!
//! # 覆盖不到的部分
//!
//! - **零周期 panic**：`run_case` 在独立线程里跑，panic 会被记成「超时」而不是
//!   「如期 panic」，因此这条留在各后端的单测里（三个后端各有一条）；
//! - **落后时的行为**：跳过还是补齐属于各后端可自行选择的部分（见
//!   `abs_art::time` 的模块文档），因此这里的用例**不**在落后态下做判定。

use core::time::Duration;

use std::time::Instant;

// `TrTime: TrDelay + TrClock`，因此只要 `R: TrTime` 在作用域内，`rt.delay(..)` 与
// `rt.now()` 都能直接解析（超 trait 的方法随之可用），不必再单独导入那两个 trait。
use abs_art::{TrInterval, TrTime};

/// 契约第 1 条：`delay` 不早于 `duration` 返回。
///
/// # Errors
///
/// `delay` 提前返回时给出实际耗时。
pub async fn probe_sleep_waits_at_least<R: TrTime>(rt: &R) -> Result<Duration, String> {
    const WAIT: Duration = Duration::from_millis(20);
    let started = Instant::now();
    rt.delay(WAIT).await;
    let elapsed = started.elapsed();
    if elapsed < WAIT {
        return Err(format!("delay({WAIT:?}) 只用了 {elapsed:?}：提前返回了"));
    }
    Ok(elapsed)
}

/// 契约第 1 条（补充）：`delay(0)` **立即**就绪。
///
/// 这不是细枝末节：「等到一个已经过去的时刻」在本家族里就是
/// `delay(deadline.saturating_duration_since(rt.now()))`，若 `delay(0)` 会阻塞，
/// 那个惯用法就不成立。因此把它钉成契约。
///
/// # Errors
///
/// `delay(0)` 耗时超过 100 ms（宽松上界，只抓结构性阻塞）时给出实际耗时。
pub async fn probe_zero_delay_completes_immediately<R: TrTime>(rt: &R) -> Result<Duration, String> {
    let started = Instant::now();
    rt.delay(Duration::ZERO).await;
    let elapsed = started.elapsed();
    if elapsed >= Duration::from_millis(100) {
        return Err(format!("delay(0) 用了 {elapsed:?}：应当立即就绪"));
    }
    Ok(elapsed)
}

/// 契约第 2 条：周期源的**第一次** `tick` 立即完成。
///
/// # Errors
///
/// 首次 tick 耗时达到或超过一个周期量级时给出实际耗时。
pub async fn probe_interval_first_tick_is_immediate<R: TrTime>(rt: &R) -> Result<Duration, String> {
    // 周期取 5 秒：首次若被推迟，本用例会一直等到 5 秒（远超阈值）才返回。
    let mut period = rt.interval(Duration::from_secs(5));
    let started = Instant::now();
    period.tick().await;
    let elapsed = started.elapsed();
    if elapsed >= Duration::from_secs(1) {
        return Err(format!("首次 tick 用了 {elapsed:?}：应当立即完成"));
    }
    Ok(elapsed)
}

/// 契约第 3 条：周期源**锚定在构造时刻**，而不是「上一次 tick 之后再等一个 period」。
///
/// # Errors
///
/// 第二次 tick 在「越过一个半周期」之后仍要等满大半个周期时给出实际耗时。
pub async fn probe_interval_is_anchored<R: TrTime>(rt: &R) -> Result<Duration, String> {
    const PERIOD: Duration = Duration::from_millis(80);
    // 越过 1.5 个周期：锚定（或跳过错过的时刻）的实现在此刻已经到点，第二次 tick
    // 应当立即返回；「每响之后再等一个周期」的实现则要再等满 ~1 个周期。
    const OVERSHOOT: Duration = Duration::from_millis(120);
    // 阈值取 0.75 个周期：锚定 ≈ 0，跳过错过的时刻 ≈ 0.5 个周期，两者都过；
    // 「每响之后再等一个周期」≈ 1 个周期，判失败。
    const LIMIT: Duration = Duration::from_millis(60);

    let mut period = rt.interval(PERIOD);
    period.tick().await; // 立即
    rt.delay(OVERSHOOT).await;

    let started = Instant::now();
    period.tick().await;
    let elapsed = started.elapsed();
    if elapsed >= LIMIT {
        return Err(format!(
            "越过 {OVERSHOOT:?} 后第二次 tick 又等了 {elapsed:?}：周期源没有锚定在构造时刻"
        ));
    }
    Ok(elapsed)
}

/// 契约第 4 条（v0.4 新增）：`now()` 单调不减，且与 `delay()` **同源**。
///
/// # 为什么这条契约是值化之后才能写的
///
/// 同源这件事在 v0.3 无法用类型表达：时刻是消费方配置项，计时器是后端类型，
/// 两者只能靠约定一致（tokio `start_paused` 下「睡在虚拟时钟、读在墙上时钟」的
/// 实测反例见 `dev-notes/runtime-tag-scope-clock-20261006-0923.md` §3.3）。
/// v0.4 起 `TrTime: TrDelay + TrClock`，两者由**同一个运行时值**提供，因此这条
/// 契约可以被直接判定。
///
/// - **手段**：同一个运行时值上①连读三次 `now()`；②记下 `start = now()`，
///   `delay(30ms)`，再读 `after = now()`；
/// - **判定**：①三次读数单调不减；②`after − start >= 30ms`（用
///   `TrClock::Instant` 的 `Sub<Self, Output = Duration>` 结构约束折算）。
///   只要计时器与 `now()` 不同源（例如一个走虚拟时钟、一个走墙上时钟），第二条
///   要么前进量不足、要么根本不前进，判失败。
///
/// # Errors
///
/// `now()` 出现回退，或 `delay` 之后 `now()` 的前进量小于所睡的时长。
pub async fn probe_clock_is_monotonic_and_shares_delay_source<R: TrTime>(
    rt: &R,
) -> Result<Duration, String> {
    const WAIT: Duration = Duration::from_millis(30);

    // ① 单调不减：连续读三次，后一次不得早于前一次。
    // 注意 `TrClock::Instant` 只要求 `Copy + Ord`，不要求 `Debug`，因此这里不打印时刻值。
    let first = rt.now();
    let second = rt.now();
    let third = rt.now();
    if second < first {
        return Err("now() 出现回退：第二次读数早于第一次".to_owned());
    }
    if third < second {
        return Err("now() 出现回退：第三次读数早于第二次".to_owned());
    }

    // ② 同源：睡 WAIT 之后，now() 至少前进 WAIT。
    let start = rt.now();
    rt.delay(WAIT).await;
    let after = rt.now();
    let advanced = after - start;
    if advanced < WAIT {
        return Err(format!(
            "delay({WAIT:?}) 之后 now() 只前进了 {advanced:?}：时刻源与计时器不同源"
        ));
    }
    Ok(advanced)
}

/// 契约第 6 条（内层赢）：内层 future 先完成时 `timeout` 原样返回输出。
///
/// # Errors
///
/// 返回了 `Elapsed`（本不该超时）时给出提示。
pub async fn probe_timeout_inner_wins<R: TrTime>(rt: &R) -> Result<u8, String> {
    match rt.timeout(Duration::from_millis(50), async { 7u8 }).await {
        Ok(v) => Ok(v),
        Err(_) => Err("内层立刻完成，却报了超时".to_owned()),
    }
}

/// 契约第 6 条（期限先到）：内层挂起时 `timeout` 在期限附近返回 `Elapsed`。
///
/// # Errors
///
/// 未在宽松上界内返回、或错误文案不符时给出提示。
pub async fn probe_timeout_elapses<R: TrTime>(rt: &R) -> Result<Duration, String> {
    const LIMIT: Duration = Duration::from_millis(30);
    let started = Instant::now();
    let outcome = rt.timeout(LIMIT, core::future::pending::<u8>()).await;
    let elapsed = started.elapsed();
    let elapsed_err = match outcome {
        Ok(_) => return Err("内层永不就绪，却不该成功".to_owned()),
        Err(elapsed_err) => elapsed_err,
    };
    if elapsed < LIMIT {
        return Err(format!("{LIMIT:?} 的期限只用了 {elapsed:?}：提前超时了"));
    }
    // 文案是跨后端的统一契约的一部分：三个后端共用 `abs_art::Elapsed`。
    if elapsed_err.to_string() != "期限已到" {
        return Err(format!("超时文案变了：{elapsed_err}"));
    }
    Ok(elapsed)
}
