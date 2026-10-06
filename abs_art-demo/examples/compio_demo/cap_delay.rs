//! # 设计意图
//!
//! 用**计时 + 时刻**两位能力 `Runtime<{ DELAY | CLOCK }>` 验证「时间驱动」能力
//! （compio 演示组，与 `examples/tokio_demo/cap_delay.rs` 一一对应）：
//!
//! 1. **`TrDelay` 作为独立能力**：`delay` 返回一个「等待指定时长后完成」的
//!    future，它只要求 `DELAY` 这一位；而**读时刻**是**另一位**能力
//!    （`CLOCK`，`1 << 5`）——「能等」与「能读表现在几点」是两件事；
//! 2. **时间抽象与后端解耦**：业务代码只依赖 `TrDelay` / `TrClock`，不感知
//!    tokio 的 `time::sleep` / compio 的 `runtime::time::sleep` / smol 的
//!    `Timer`；
//! 3. **时刻与计时同源**：`TrClock::now()` 也从**同一个运行时值**上取。compio
//!    侧的计时基准同样是 `std::time::Instant`，因此 `now()` 与 `delay` 看的是
//!    同一个钟。代价是连锁的：`TrTime: TrDelay + TrClock`，所以
//!    `delay` / `now` / `timeout` 三者齐备的最小声明是 `{ DELAY | CLOCK }`，
//!    只写 `DELAY` 时 `now()` 会被编译期拒绝（`HasClock` 不满足）；
//! 4. 连续多次 `delay` 可以串联成周期性节奏（tick）。
//!
//! # 计时属于运行时值，不属于作用域
//!
//! `delay` / `now` 是**运行时值**的方法，作用域值上没有它们——这与 tokio 组
//! 完全一致：抽象层把「计时」与「本地投递」分给两个不同的宿主。
//!
//! 一条 compio 侧的如实记录：compio 的计时注册本身是**环境式**的（`time::sleep`
//! 靠线程本地上下文找运行时，且注册发生在首次轮询），因此注册点落在**轮询所在
//! 线程的当前 compio 运行时**上；在本示例里，外层 `rt.block_on` 与值指向的是
//! 同一份运行时，两者一致。这条后端差异见 `abs_art-compio` 的 crate 文档。
//!
//! # 可以做到
//!
//! - `delay(duration)` 产生一个 future，`await` 它至少等待 `duration`；
//! - `now()` 从同一个值上读时刻；
//! - 多次 `delay` 串联/循环，构造节拍（tick）；
//! - 与其它能力自由组合（组合方式见 `cap_full`）。
//!
//! # 不能做到
//!
//! - **time driver 未启用时** `delay` 的 future 会永远 pending——compio 的
//!   `Runtime::new()` **默认开启全部 driver**（含 time），因此本示例直接可用；
//!   但这是创建运行时一方的责任，抽象层不替你开 driver；
//! - `delay` 本身不提供「定时回调」：它只是睡眠，要在指定时间执行逻辑需要
//!   与投递能力组合，那是另一项能力的声明；
//! - 不保证「精确到期」：睡眠语义是「至少等待这么久」，不是硬实时时钟；
//! - 不能在作用域值上调用 `delay` / `now`：那两个方法根本不在 `LocalScope` 上。

use std::time::{Duration, Instant};

use bridge_compio::{CLOCK, DELAY, CompioRuntime as Runtime, TrClock, TrDelay};

/// 能力声明：`DELAY | CLOCK` **两位**——`delay` 只要 `DELAY`，而 `now()` 要
/// `CLOCK`（读时刻是独立的一位能力，不复用 `DELAY`）。
///
/// compio 的静态断言 `CompioCaps_` 覆盖这个掩码（`34`，不含 `SPAWN_SEND` 位），
/// 因此构造与使用都不受额外限制。本示例里 `clock_is_monotonic_` 会调
/// `rt.now()`；若只写 `DELAY`，那行会以「`TrClock` 的 impl 条件不满足」被编译期
/// 拒绝。
type DelayRt = Runtime<{ DELAY | CLOCK }>;

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
fn clock_is_monotonic_(rt: &DelayRt) -> bool {
    let before = rt.now();
    let after = rt.now();
    after >= before
}

fn main() {
    // compio：Runtime::new() 默认开启 time driver，无需像 tokio 那样 enable_all
    let rt = compio::runtime::Runtime::new().unwrap();

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
    println!("compio cap_delay OK: elapsed={elapsed}ms, ticks={ticks}, monotonic={monotonic}");
}
