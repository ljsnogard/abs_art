//! # 设计意图
//!
//! 用**单能力** `Runtime<{ DELAY }>` 验证「时间驱动」能力（compio 演示组，
//! 与 `examples/tokio_demo/cap_delay.rs` 一一对应）：
//!
//! 1. **`TrDelay` 作为独立能力**：`delay` 返回一个「等待指定时长后完成」的
//!    future，能力声明可以只要 `DELAY`，不声明任何其它能力；
//! 2. **时间抽象与后端解耦**：业务代码只依赖 `TrDelay::delay`，不感知
//!    tokio 的 `time::sleep` / compio 的 `runtime::time::sleep` / smol 的
//!    `Timer`；
//! 3. 连续多次 `delay` 可以串联成周期性节奏（tick）。
//!
//! # 可以做到
//!
//! - `delay(duration)` 产生一个 future，`await` 它至少等待 `duration`；
//! - 多次 `delay` 串联/循环，构造节拍（tick）；
//! - 与其它能力自由组合（例如 `SPAWN_SEND` 组合出「定时执行」语义——
//!   组合方式见 `cap_full`，那里五种能力协同）。
//!
//! # 不能做到
//!
//! - **time driver 未启用时** `delay` 的 future 会永远 pending——compio 的
//!   `Runtime::new()` **默认开启全部 driver**（含 time），因此本示例直接
//!   可用；但这是创建运行时一方的责任，抽象层不替你开 driver（若换成
//!   `RuntimeBuilder` 只选部分 driver，就需要自己保证 time 开启）；
//! - `delay` 本身不提供「定时回调」：它只是睡眠，要在指定时间执行逻辑需要
//!   与 `spawn`（`SPAWN_SEND`）组合，那是另一项能力的声明；
//! - 不保证「精确到期」：睡眠语义是「至少等待这么久」，不是硬实时时钟。

use std::time::{Duration, Instant};

use bridge_compio::{DELAY, Runtime, TrDelay};

/// 能力声明：只请求 `delay` 一种能力（最小权限）。
type DelayRt = Runtime<{ DELAY }>;

/// 业务函数 A：睡眠至少 `ms` 毫秒，返回实际经过的毫秒数。
async fn wait_at_least(ms: u64) -> u128 {
    let start = Instant::now();
    // TrDelay::delay 返回一个等待 duration 之后完成的 future
    <DelayRt as TrDelay>::delay(Duration::from_millis(ms)).await;
    start.elapsed().as_millis()
}

/// 业务函数 B：连续三次短睡眠，构造 3 个 tick 的节拍。
async fn tick_tock() -> usize {
    let mut ticks = 0;
    for _ in 0..3 {
        <DelayRt as TrDelay>::delay(Duration::from_millis(1)).await;
        ticks += 1;
    }
    ticks
}

fn main() {
    // compio：Runtime::new() 默认开启 time driver，无需像 tokio 那样 enable_all
    let rt = compio::runtime::Runtime::new().unwrap();

    let (elapsed, ticks) = rt.block_on(async {
        let elapsed = wait_at_least(10).await;
        let ticks = tick_tock().await;
        (elapsed, ticks)
    });

    assert!(elapsed >= 10, "实际只等了 {elapsed}ms");
    assert_eq!(ticks, 3);
    println!("compio cap_delay OK: elapsed={elapsed}ms, ticks={ticks}");
}
