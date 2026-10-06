//! `time`：计时能力——绝对时刻（[`TrClock`]）、周期源（[`TrInterval`]）与超时（[`TrTime`]）。
//!
//! 实现基于 compio 的 `runtime::time`。本 crate 是家族里**唯一需要 nightly** 的：
//! compio 的睡眠 / 周期 / 超时 future 全是 `pub async fn`（不透明、不可命名），
//! 关联类型只能用 ITIT（`impl_trait_in_assoc_type`）给出。因果与取舍见
//! `dev-notes/time-20261005-1225.md` §11。
//!
//! compio 的周期语义（首次立即 + 相位对齐 + 落后时跳过）正是 `abs_art::time` 契约的
//! 原型，因此周期源**只做包裹**，不额外改写。
//!
//! 超时不必在这里实现：[`TrTime::timeout`] 是 trait 的默认方法，返回 `abs_art` 的
//! [`Timeout`](abs_art::Timeout)。
//!
//! 与 `delay` 共用 `delay` feature：两者要的是同一个运行时 feature（compio 的
//! `time`），没有拆开的必要。
//!
//! # [`TrClock::Instant`] 为什么是 [`std::time::Instant`]
//!
//! 「与计时器同源」是硬要求：tokio 后端给 `tokio::time::Instant`（`test-util` 下
//! 可暂停），compio 后端则必须给出 compio 计时器所用的那个时刻类型。已实测
//! （`compio-runtime-0.12.6/src/time/mod.rs`）：compio 的 `sleep_until` /
//! `interval_at` / `Interval::tick` 全部直接使用 `std::time::Instant`，且**没有**
//! 导出任何自己的时刻类型（该模块只 `pub use future::Interval;`）。因此
//! `type Instant = std::time::Instant` 是唯一「同源」的选择，而不是退而求其次。
//!
//! 代价是本 crate 必须显式 `extern crate std;`（见 [crate 文档](crate)）；
//! `std::time::Instant` 满足 `TrClock::Instant` 的四个结构约束
//! （`Copy + Ord`、`Add<Duration, Output = Instant>`、`Sub<Instant, Output = Duration>`、
//! `'static`），编译期已验证。
//!
//! # `CLOCK` 门控：没声明就没有 `now()`
//!
//! 读时刻是**独立的一位能力**（[`CLOCK`](abs_art::CLOCK)），不复用
//! [`DELAY`](abs_art::DELAY)：`impl TrClock for Runtime<CAPS>` 要求
//! `[(); CAPS]: HasClock`，因此只写 `DELAY` 的运行时值**没有** `now()`：
//!
//! ```compile_fail
//! use abs_art::DELAY;
//! use abs_art_compio::Runtime;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let value = Runtime::<{ DELAY }>::current();
//!     // 没写 CLOCK → 没有 now()
//!     // 实测原文（E0599）：no method named `now` found for struct
//!     // `abs_art_compio::Runtime<2>` in the current scope
//!     let _ = value.now();
//! });
//! ```
//!
//! 更值得注意的是**同源约束的连带后果**：`TrTime: TrDelay + TrClock`
//! （[`abs_art::time`] 的定义），而本 crate 的 `TrTime` impl 也门控在这两个位上，于是
//! **`interval` / `timeout` 同样要声明 `CLOCK`**——明明只想要周期源，也得承认自己
//! 依赖同一个钟：
//!
//! ```compile_fail
//! use abs_art::DELAY;
//! use abs_art_compio::Runtime;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let value = Runtime::<{ DELAY }>::current();
//!     // TrTime: TrDelay + TrClock → 要 interval 就得同时有 CLOCK
//!     // 实测原文（E0599）：no method named `interval` found for struct
//!     // `abs_art_compio::Runtime<2>` in the current scope
//!     let _ = value.interval(core::time::Duration::from_millis(1));
//! });
//! ```
//!
//! 反过来，`DELAY | CLOCK` 就够用（`delay` / `now` / `interval` / `timeout` 齐备），
//! 后缀 `timeout` 走的还是 `TrTime` 的默认方法：
//!
//! ```
//! use abs_art::{CLOCK, DELAY, TrClock, TrDelay, TrInterval, TrTime};
//! use abs_art_compio::Runtime;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! let out = rt.block_on(async {
//!     let value = Runtime::<{ DELAY | CLOCK }>::current();
//!     let before = value.now();
//!     value.delay(core::time::Duration::from_millis(1)).await;
//!     let mut period = value.interval(core::time::Duration::from_millis(1));
//!     period.tick().await;
//!     let timed = value
//!         .timeout(core::time::Duration::from_millis(1), async { 42u8 })
//!         .await
//!         .unwrap_or(0);
//!     assert!(value.now() - before >= core::time::Duration::from_millis(1));
//!     timed
//! });
//! assert_eq!(out, 42);
//! ```

use core::{future::Future, time::Duration};

use abs_art::{HasClock, HasDelay, TrClock, TrInterval, TrTime};

use crate::{CompioCaps_, Runtime};

/// 本后端的周期源（[`TrTime::Interval`] 的具体类型）。
#[derive(Debug)]
pub struct Interval {
    /// compio 自己的周期源；它已经满足 `abs_art::time` 的语义契约，故只做包裹。
    inner_: compio::runtime::time::Interval,
}

impl TrInterval for Interval {
    /// compio 的 `Interval::tick` 是 `async fn`（不透明），用 ITIT 命名它。
    type Tick<'a> = impl Future<Output = ()> + 'a;

    fn tick(&mut self) -> Self::Tick<'_> {
        async move {
            // compio 的 `tick` 返回到期时刻（`std::time::Instant`）；本 crate 的
            // `TrInterval` 契约不暴露 tick 对应的时刻（见 `abs_art::time` 模块
            // 文档），因此丢弃返回值。
            self.inner_.tick().await;
        }
    }
}

/// [`TrClock`] 的门控是**独立的一位** [`CLOCK`](abs_art::CLOCK)：写 `DELAY` 不等于
/// 写 `CLOCK`，只读表不等待的后端/调用方可以只写后者。
impl<const CAPS: usize> TrClock for Runtime<CAPS>
where
    [(); CAPS]: HasClock,
    [(); CAPS]: CompioCaps_,
{
    /// 与 [`TrDelay`](abs_art::TrDelay) 的计时器**同一时间基准**：
    /// `compio::runtime::time` 的 `sleep_until` / `interval_at` 形参就是
    /// [`std::time::Instant`]，驱动也用它折算 `current_timeout`。
    ///
    /// 注意 compio 没有可暂停的虚拟时钟（对照 tokio 的 `test-util`），因此
    /// `now()` 永远是墙上时钟。
    type Instant = std::time::Instant;

    fn now(&self) -> Self::Instant {
        std::time::Instant::now()
    }
}

/// [`TrTime: TrDelay + TrClock`](TrTime)：本 impl 必须同时门控
/// `[(); CAPS]: HasDelay`（满足 `TrDelay` 超 trait）与 `[(); CAPS]: HasClock`
/// （满足 `TrClock` 超 trait），所以 `interval` / `timeout` **要 CLOCK 与 DELAY
/// 两个位**——这是「与计时器同源」那条结构约束的直接后果，不是额外要求。
impl<const CAPS: usize> TrTime for Runtime<CAPS>
where
    [(); CAPS]: HasDelay,
    [(); CAPS]: HasClock,
    [(); CAPS]: CompioCaps_,
{
    type Interval = Interval;

    /// 构造 compio 的周期源：首次 tick 立即完成，进度锚定在构造时刻，落后时跳过。
    ///
    /// `period` 为零时 compio 自身 panic（文案 `` `period` must be non-zero. ``），
    /// 与 `abs_art::time` 的契约第 5 条一致，故不再重复断言。
    fn interval(&self, period: Duration) -> Self::Interval {
        Interval {
            inner_: compio::runtime::time::interval(period),
        }
    }
}

#[cfg(test)]
mod tests {
    //! compio 后端的 `TrClock` / `TrTime` 单测。
    //!
    //! 跨三后端的一致性契约由 `abs_art-smoke` 的 `time_contract` 用例负责；这里只
    //! 钉住「本后端能不能跑起来」与「时刻与计时器是否同源」这几条。

    use core::time::Duration;
    // 本 crate 是 `no_std`，`core` 的 prelude 里没有 `ToString`；经
    // `extern crate std;` 取它，免得为了断言文案引入 `alloc`。
    use std::{string::ToString, time::Instant};

    use abs_art::{TrClock, TrDelay, TrInterval, TrTime};

    use crate::Runtime;

    /// 建一个 compio 运行时。
    fn rt_() -> compio::runtime::Runtime {
        compio::runtime::Runtime::new().expect("建 compio 运行时")
    }

    /// 目的：验证 `delay` 能真正睡到（compio 的 time 驱动被驱动）。
    ///
    /// 实施策略：在 compio 运行时里构造运行时值，await 1 ms 的 `value.delay`，
    /// 量测实际耗时。
    ///
    /// 通过依据：正常返回且耗时 `>= 1 ms`；若 time 驱动没被驱动，本用例会挂死。
    #[test]
    fn delay_completes_and_waits_at_least_the_duration() {
        let rt = rt_();
        rt.block_on(async {
            let value = crate::current();
            let started = Instant::now();
            value.delay(Duration::from_millis(1)).await;
            assert!(
                started.elapsed() >= Duration::from_millis(1),
                "delay 不该提前返回"
            );
        });
    }

    /// 目的：验证 [`TrClock::now`] 与计时器**同源**——即 `now()` 走的钟就是
    /// `delay` 等待用的那个钟（不是另找一个「差不多」的时钟）。
    ///
    /// 实施策略：在同一运行时值上先读 `now()`，再用 `delay` 睡 5 ms，最后再读
    /// `now()`，用 `TrClock::Instant` 的 `Sub` 计算两者之差。
    ///
    /// 通过依据：差值 `>= 5 ms`。若 `Instant` 与计时器不同源（例如一个用墙上
    /// 时钟、一个用单调计数），差值会明显偏离等待时长。
    #[test]
    fn clock_now_shares_the_timer_basis() {
        let rt = rt_();
        rt.block_on(async {
            let value = crate::current();
            let before = value.now();
            value.delay(Duration::from_millis(5)).await;
            let after = value.now();
            assert!(
                after - before >= Duration::from_millis(5),
                "now() 与 delay 的钟不同源：差值为 {:?}",
                after - before
            );
        });
    }

    /// 目的：验证周期源的**第一次** tick 立即完成（契约第 2 条）。
    ///
    /// 实施策略：取一个 5 秒的周期，量测第一次 tick 的耗时。
    ///
    /// 通过依据：耗时远小于周期（这里取 `< 1 秒`）——compio 的 `interval` 起点取
    /// 「现在」，因此首次 `tick` 立即就绪；若被推迟一个周期，本用例会等 5 秒。
    #[test]
    fn interval_first_tick_is_immediate() {
        let rt = rt_();
        rt.block_on(async {
            let value = crate::current();
            let mut period = value.interval(Duration::from_secs(5));
            let started = Instant::now();
            period.tick().await;
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "第一次 tick 应当立即完成"
            );
        });
    }

    /// 目的：验证零周期在构造点被拒绝（契约第 5 条）。
    ///
    /// 实施策略：`#[should_panic]` 捕获 `value.interval(Duration::ZERO)`——compio
    /// 自身在 `interval_at` 上就会断言。
    ///
    /// 通过依据：panic 文案含 "zero"。
    #[test]
    #[should_panic(expected = "zero")]
    fn interval_rejects_zero_period() {
        let rt = rt_();
        rt.block_on(async {
            let value = crate::current();
            let _ = value.interval(Duration::ZERO);
        });
    }

    /// 目的：验证 `DELAY | CLOCK` 两个位就凑齐全部计时能力——`now`（只要 `CLOCK`）、
    /// `delay`（只要 `DELAY`）、`interval` 与 `timeout`（`TrTime`，两个位都要）。
    ///
    /// 实施策略：在 compio 运行时上下文内用 `Runtime::<{ DELAY | CLOCK }>::current()`
    /// 构造运行时值，依次读 `now()`、`delay` 睡 1 ms、取 `interval` 并 tick 一次、
    /// 对已就绪的 future 施加 1 ms `timeout`。
    ///
    /// 通过依据：`now()` 的两次读值之差 `>= 1 ms`（钟与 delay 同源）、首次 tick 立即
    /// 返回（`< 1 s`）、`timeout` 返回 `Ok(42)`。若 `HasClock` 门控写错（例如仍挂在
    /// `HasDelay` 上），本用例里 `now()` 依旧可用但门控含义失真，编译期正例由
    /// `abs_art::caps` 侧的 32 掩码表与模块文档里的 `compile_fail` 用例共同钉住。
    #[test]
    fn delay_and_clock_give_full_time_capabilities() {
        use abs_art::{CLOCK, DELAY};

        let rt = rt_();
        rt.block_on(async {
            let value = Runtime::<{ DELAY | CLOCK }>::current();

            let before = value.now();
            value.delay(Duration::from_millis(1)).await;
            assert!(
                value.now() - before >= Duration::from_millis(1),
                "now() 与 delay 的钟不同源：差值为 {:?}",
                value.now() - before
            );

            let mut period = value.interval(Duration::from_secs(5));
            let started = Instant::now();
            period.tick().await;
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "首次 tick 应立即可用"
            );

            let out = value
                .timeout(Duration::from_millis(1), async { 42u8 })
                .await;
            assert_eq!(out.expect("已就绪的 future 不该超时"), 42u8);
        });
    }

    /// 目的：验证 `TrTime::timeout`（trait 默认方法）在本后端可用，且期限先到时
    /// 返回 `Elapsed`。
    ///
    /// 实施策略：用运行时值对一个永不就绪的 future 施加 5 ms 超时。
    ///
    /// 通过依据：结果为 `Err`，且 `Display` 文案为「期限已到」。
    #[test]
    fn timeout_elapses_on_a_pending_inner_future() {
        let rt = rt_();
        rt.block_on(async {
            let value = crate::current();
            let out = value
                .timeout(Duration::from_millis(5), core::future::pending::<u8>())
                .await;
            let elapsed = out.expect_err("期限已到应当是错误");
            assert_eq!(elapsed.to_string(), "期限已到");
        });
    }
}
