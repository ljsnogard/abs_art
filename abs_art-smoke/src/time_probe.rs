//! 跨后端 [`TrTime`] 行为契约的测试体（**泛型于 `T: TrTime`**）。
//!
//! 与 `spawn_local` 那组同形：测试体只有**一份**，三个后端各拿它跑一遍，于是
//! 「同一份 trait、三个实现」最容易出的**语义漂移**变成可执行的判定。
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

use abs_art::{TrInterval, TrTime};

/// 契约第 1 条：`delay` 不早于 `duration` 返回。
///
/// # Errors
///
/// `sleep` 提前返回时给出实际耗时。
pub async fn probe_sleep_waits_at_least<T: TrTime>() -> Result<Duration, String> {
    const WAIT: Duration = Duration::from_millis(20);
    let started = Instant::now();
    T::delay(WAIT).await;
    let elapsed = started.elapsed();
    if elapsed < WAIT {
        return Err(format!("sleep({WAIT:?}) 只用了 {elapsed:?}：提前返回了"));
    }
    Ok(elapsed)
}

/// 契约第 1 条（补充）：`delay(0)` **立即**就绪。
///
/// 这不是细枝末节：「等到一个已经过去的时刻」在本家族里就是
/// `delay(deadline.saturating_duration_since(clock.now()))`，若 `delay(0)` 会阻塞，
/// 那个惯用法就不成立。因此把它钉成契约。
///
/// # Errors
///
/// `delay(0)` 耗时超过 100 ms（宽松上界，只抓结构性阻塞）时给出实际耗时。
pub async fn probe_zero_delay_completes_immediately<T: TrTime>() -> Result<Duration, String> {
    let started = Instant::now();
    T::delay(Duration::ZERO).await;
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
pub async fn probe_interval_first_tick_is_immediate<T: TrTime>() -> Result<Duration, String> {
    // 周期取 5 秒：首次若被推迟，本用例会一直等到 5 秒（远超阈值）才返回。
    let mut period = T::interval(Duration::from_secs(5));
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
pub async fn probe_interval_is_anchored<T: TrTime>() -> Result<Duration, String> {
    const PERIOD: Duration = Duration::from_millis(80);
    // 越过 1.5 个周期：锚定（或跳过错过的时刻）的实现在此刻已经到点，第二次 tick
    // 应当立即返回；「每响之后再等一个周期」的实现则要再等满 ~1 个周期。
    const OVERSHOOT: Duration = Duration::from_millis(120);
    // 阈值取 0.75 个周期：锚定 ≈ 0，跳过错过的时刻 ≈ 0.5 个周期，两者都过；
    // 「每响之后再等一个周期」≈ 1 个周期，判失败。
    const LIMIT: Duration = Duration::from_millis(60);

    let mut period = T::interval(PERIOD);
    period.tick().await; // 立即
    T::delay(OVERSHOOT).await;

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

/// 契约第 6 条（内层赢）：内层 future 先完成时 `timeout` 原样返回输出。
///
/// # Errors
///
/// 返回了 `Elapsed`（本不该超时）时给出提示。
pub async fn probe_timeout_inner_wins<T: TrTime>() -> Result<u8, String> {
    match T::timeout(Duration::from_millis(50), async { 7u8 }).await {
        Ok(v) => Ok(v),
        Err(_) => Err("内层立刻完成，却报了超时".to_owned()),
    }
}

/// 契约第 6 条（期限先到）：内层挂起时 `timeout` 在期限附近返回 `Elapsed`。
///
/// # Errors
///
/// 未在宽松上界内返回、或错误文案不符时给出提示。
pub async fn probe_timeout_elapses<T: TrTime>() -> Result<Duration, String> {
    const LIMIT: Duration = Duration::from_millis(30);
    let started = Instant::now();
    let outcome = T::timeout(LIMIT, core::future::pending::<u8>()).await;
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
