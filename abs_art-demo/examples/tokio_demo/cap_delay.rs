//! # 设计意图
//!
//! 用**单能力** `Runtime<{ DELAY }>` 验证「时间驱动」能力：
//!
//! 1. **`TrDelay` 作为独立能力**：`delay` 返回一个「等待指定时长后完成」的
//!    future，能力声明可以只要 `DELAY`，不声明任何其它能力；
//! 2. **时间抽象与后端解耦**：业务代码只依赖 `TrDelay` / `TrClock`，不感知
//!    tokio 的 `time::sleep` / smol 的 `Timer` / compio 的时间设施；
//! 3. **时刻与计时同源**：`TrClock::now()` 也从**同一个运行时值**上取——这正是
//!    把时刻做成 `TrTime` 超 trait 的目的（避免「睡在虚拟时钟上、读在墙上时钟
//!    上」这类错配）；
//! 4. 连续多次 `delay` 可以串联成周期性节奏（tick）。
//!
//! # 计时属于运行时值，不属于作用域
//!
//! `delay` / `now` 是**运行时值**的方法，作用域值上**没有**它们（作用域只回答
//! 「`!Send` 任务投到哪、由谁驱动」）。因此本示例不需要 `rt.local_scope()`，
//! 也不涉及任何本地队列。
//!
//! # 可以做到
//!
//! - `delay(duration)` 产生一个 future，`await` 它至少等待 `duration`；
//! - `now()` 从同一个值上读时刻（`TrClock::Instant` 是关联类型，后端可换基准）；
//! - 多次 `delay` 串联/循环，构造节拍（tick）；
//! - 与其它能力自由组合（组合方式见 `cap_full`）。
//!
//! # 不能做到
//!
//! - **time driver 未启用时**（tokio 后端）`delay` 的 future 会永远 pending
//!   （`tokio::time` 需要运行时开启 `enable_time` / `enable_all`）——这是
//!   集成方创建运行时时的责任，抽象层不替你开 driver；
//! - `delay` 本身不提供「定时回调」：它只是睡眠，要在指定时间执行逻辑需要
//!   与 `spawn`（`SPAWN_SEND`）组合，那是另一项能力的声明；
//! - 不保证「精确到期」：睡眠语义是「至少等待这么久」，不是硬实时时钟；
//! - 不能在作用域值上调用 `delay` / `now`：那两个方法根本不在 `LocalScope` 上。

use std::time::{Duration, Instant};

use bridge_tokio::{DELAY, TokioRuntime as Runtime, TrClock, TrDelay};

/// 能力声明：只请求 `delay` 一位——按抽象层的设计，这一位同时覆盖
/// `TrDelay`（睡眠）与 `TrClock`（时刻）。
type DelayRt = Runtime<{ DELAY }>;

/// 业务函数 A：在这个运行时值上睡眠至少 `ms` 毫秒，返回实际经过的毫秒数。
async fn wait_at_least_(rt: &DelayRt, ms: u64) -> u128 {
    let start = Instant::now();
    // TrDelay::delay 返回一个等待 duration 之后完成的 future
    rt.delay(Duration::from_millis(ms)).await;
    start.elapsed().as_millis()
}

/// 业务函数 B：连续三次短睡眠，构造 3 个 tick 的节拍。
async fn tick_tock_(rt: &DelayRt) -> usize {
    let mut ticks = 0;
    for _ in 0..3 {
        rt.delay(Duration::from_millis(1)).await;
        ticks += 1;
    }
    ticks
}

/// 业务函数 C：时刻也从**同一个运行时值**上取，且单调不减。
///
/// `R::Instant` 是关联类型：tokio 给出 `tokio::time::Instant`（与它的计时器同一
/// 时间基准），compio / smol 给出 `std::time::Instant`。业务侧只依赖
/// `TrClock` 的三条结构约束（`Copy + Ord`、可与 `Duration` 加减），因此换后端
/// 不需要改这里一行。
fn clock_is_monotonic_(rt: &DelayRt) -> bool {
    let before = rt.now();
    let after = rt.now();
    after >= before
}

fn main() {
    // tokio 后端要求 time driver 已开启：enable_all（等价于 Runtime::new()）
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let (elapsed, ticks, monotonic) = rt.block_on(async {
        // 在运行时上下文内构造值：类型别名已固定 CAPS，无需 turbofish
        let value = DelayRt::current();
        let elapsed = wait_at_least_(&value, 10).await;
        let ticks = tick_tock_(&value).await;
        let monotonic = clock_is_monotonic_(&value);
        (elapsed, ticks, monotonic)
    });

    assert!(elapsed >= 10, "实际只等了 {elapsed}ms");
    assert_eq!(ticks, 3);
    assert!(monotonic, "同一个值上的时刻必须单调不减");
    println!("cap_delay OK: elapsed={elapsed}ms, ticks={ticks}, monotonic={monotonic}");
}
