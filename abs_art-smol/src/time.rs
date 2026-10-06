//! `time`：计时能力——时刻（[`TrClock`]）、周期源（[`TrInterval`]）与超时（[`TrTime`]）。
//!
//! 实现基于 smol 的 `Timer`（即 `async_io::Timer`，由 async-io 的进程级反应器
//! 驱动）：
//!
//! - 一次性睡眠：`TrDelay::Delay` = `UnitFuture<smol::Timer>`（在 `delay.rs` 里给出）；
//! - 时刻：[`TrClock::Instant`] = `std::time::Instant`——**与上面的睡眠同一时间
//!   基准**，理由见下；
//! - 周期源：[`Interval`]——**自建**，因为 `async_io::Timer::interval` 的首次 tick
//!   落在「一个周期之后」，与本 crate 的契约（首次**立即**）不符；
//! - `tick` 的 future：[`Tick`]。
//!
//! 超时不必在这里实现：[`TrTime::timeout`] 是 trait 的默认方法，返回 `abs_art` 的
//! [`Timeout`](abs_art::Timeout)。
//!
//! 与 `delay` 共用 `delay` feature：两者要的是同一个驱动器（async-io 的反应器）。
//!
//! # 能力位：读时刻要 `CLOCK`，周期与超时要 `DELAY + CLOCK`
//!
//! [`TrClock`] 门控在 [`CLOCK`](abs_art::CLOCK) 上（`delay` 只要求
//! [`DELAY`](abs_art::DELAY)）：**能等**与**能读表现在几点**是两件事，读时刻不需要
//! 反应器跑起来。
//!
//! [`TrTime`]（`interval` / `timeout`）门控在 `DELAY` **与** `CLOCK` 两者上——这不是
//! 随手加的一条，而是「同源」那条结构约束的直接后果：`TrTime: TrDelay + TrClock`
//! 要求有周期源的值必然也能报时刻，所以**要 `interval` / `timeout` 就得同时声明
//! `CLOCK`**。只声明 `DELAY` 时 `delay` 仍然可用，而 `now()` 不可用。
//!
//! 下面一对文档测试（一负一正）钉住这条门控：
//!
//! - **目的**：只声明 `DELAY` 时 `now()` 不可用；声明 `DELAY | CLOCK` 时 `now()` 与
//!   `interval` 都可用。
//! - **手段**：负例在 `Runtime<{ DELAY }>` 上调用 `now()` 并标为 `compile_fail`；
//!   正例在 `Runtime<{ DELAY | CLOCK }>` 上调用同一方法，再取一个周期源。
//! - **判断**：负例编译失败、正例编译并运行成功，两者**同时**成立才算门控被正确钉住
//!   ——正例还排除了「负例因别的原因失败」这种假通过。
//!
//! ```compile_fail
//! use abs_art::DELAY;
//! use abs_art_smol::Runtime;
//!
//! // 只有 `DELAY`：`Runtime<{ DELAY }>` 没有实现 `TrClock`
//! let rt = Runtime::<{ DELAY }>::current();
//! let _ = rt.now(); // 编译失败：当前类型上找不到 `now`
//! ```
//!
//! 正例（两个位都声明，两者都可用）：
//!
//! ```
//! use abs_art::{CLOCK, DELAY, TrClock, TrTime};
//! use abs_art_smol::Runtime;
//!
//! let value = Runtime::<{ DELAY | CLOCK }>::current();
//! let _ = value.now(); // CLOCK → `TrClock` 可用
//! let _ = value.interval(core::time::Duration::from_secs(60)); // DELAY | CLOCK → `TrTime` 可用
//! ```
//!
//! # 为什么 `TrClock::Instant` 取 `std::time::Instant`
//!
//! [`TrClock`] 要求 `Instant: Copy + Ord + Add<Duration, Output = Self> +
//! Sub<Self, Output = Duration> + 'static`。`std::time::Instant` 四条全满足：
//!
//! | 约束 | 由谁提供 |
//! | --- | --- |
//! | `Copy + Ord` | `std` 的 `#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]` |
//! | `Add<Duration, Output = Instant>` | `impl Add<Duration> for Instant`（`deadline = now + budget`） |
//! | `Sub<Instant, Output = Duration>` | `impl Sub<Instant> for Instant`（`elapsed = then − now`，对更早的时刻饱和到零） |
//! | `'static` | 自有值，不含借用 |
//!
//! 更关键的是**同源**：async-io 的计时器内部就是按 `std::time::Instant` 计算的
//! （`async_io::Timer::after(d)` 即 `Instant::now() + d`；async-io 源码里
//! `use std::time::{Duration, Instant}`），而 `TrDelay::Delay` 正是那个
//! `smol::Timer` 的包装。于是本后端的 `now()` 与 `delay()` 走的是**同一个**时间
//! 基准，`TrTime: TrDelay + TrClock` 这条结构性绑定在 smol 上是真的成立的，而不是
//! 靠约定。要注入假时钟时，替换的应当是**整个运行时值**（同时实现 `TrDelay` 与
//! `TrClock`），而不是给真反应器配一个外来时刻源。
//!
//! 注意**不能**直接为 `std::time::Instant` 实现 [`TrClock`]（trait 与类型都对外来
//! crate 而言是外来的，孤儿规则 `E0117`）——所以它天然是「运行时的能力」。

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use abs_art::{HasClock, HasDelay, TrClock, TrInterval, TrTime};

use crate::Runtime;

/// 本后端的周期源（[`TrTime::Interval`] 的具体类型）。
///
/// 内部机关是 `smol::Timer::interval(period)`：它在**构造时**就把时刻锚定为
/// `now + period`，此后每响一次把时刻 `+ period`（见 `async_io` 的
/// `Stream for Timer`），因此天然是「锚定」而非「上一觉之后再等一个 period」。
/// 我们只需把它的第一次响应当作「第二次 tick」——第一次 tick 由 [`Tick`] 直接放行。
#[derive(Debug)]
pub struct Interval {
    first_: bool,
    inner_: smol::Timer,
}

/// 本后端 `tick(&mut self)` 的 future（[`TrInterval::Tick`] 的具体类型）。
#[derive(Debug)]
pub struct Tick<'a> {
    interval_: &'a mut Interval,
}

impl Future for Tick<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // `Tick` 只含 `&mut Interval`，因此是 `Unpin`，`get_mut` 可用。
        // 注意要**重借**（`&mut *…`）：直接把 `&mut` 字段取出来会试图把它移出 `self`。
        let interval = &mut *self.get_mut().interval_;
        if interval.first_ {
            interval.first_ = false;
            return Poll::Ready(());
        }
        // `smol::Timer` 是 `Unpin`：安全地借用轮询（它同时是 `Future<Output = Instant>`，
        // 到点即就绪，并按周期重挂；返回值按契约丢弃）。
        Pin::new(&mut interval.inner_).poll(cx).map(|_| ())
    }
}

impl TrInterval for Interval {
    type Tick<'a> = Tick<'a>;

    fn tick(&mut self) -> Self::Tick<'_> {
        Tick { interval_: self }
    }
}

impl<const CAPS: usize> TrClock for Runtime<CAPS>
where
    [(); CAPS]: HasClock,
{
    /// 与 [`TrDelay`](abs_art::TrDelay) 的计时器**同一时间基准**：async-io 的计时器
    /// 内部用的就是 `std::time::Instant`（见模块文档的约束表）。
    type Instant = std::time::Instant;

    /// 读取当前时刻。
    fn now(&self) -> Self::Instant {
        std::time::Instant::now()
    }
}

impl<const CAPS: usize> TrTime for Runtime<CAPS>
where
    [(); CAPS]: HasDelay,
    [(); CAPS]: HasClock,
{
    type Interval = Interval;

    fn interval(&self, period: Duration) -> Self::Interval {
        smol_interval_(period)
    }
}

/// 自建周期源：首次 `tick` 立即，其后锚定在构造时刻、每 `period` 一响。
///
/// # Panics
///
/// `period` 为零时 panic。**必须**自己断言：`async_io::Timer::interval(0)` 不会
/// panic，而是变成一个「每个时刻都立即到点」的忙循环。
fn smol_interval_(period: Duration) -> Interval {
    assert!(period > Duration::ZERO, "`period` must be non-zero.");
    Interval {
        first_: true,
        inner_: smol::Timer::interval(period),
    }
}

#[cfg(test)]
mod tests {
    //! smol 后端的 `TrDelay` / `TrClock` / `TrTime` 单测。
    //!
    //! 跨三后端的一致性契约由 `abs_art-smoke` 的 `time_contract` 用例负责；这里只
    //! 钉住「本后端能不能跑起来」这几条。

    use core::ops::{Add, Sub};

    use abs_art::{CLOCK, DELAY, FULL, TrDelay};

    use super::*;

    /// 目的：验证 `delay` 能真正睡到（async-io 的反应器被驱动）。
    ///
    /// 手段：构造 `Runtime<{ FULL }>` 值，在 `smol::block_on` 里 await 1ms 的
    /// `delay`，量测实际耗时。
    ///
    /// 判定：正常返回且耗时 `>= 1ms`；若反应器没被驱动，本用例会挂死。
    #[test]
    fn delay_completes_and_waits_at_least_the_duration() {
        let rt = Runtime::<{ FULL }>::current();
        smol::block_on(async {
            let started = std::time::Instant::now();
            rt.delay(Duration::from_millis(1)).await;
            assert!(
                started.elapsed() >= Duration::from_millis(1),
                "delay 不该提前返回"
            );
        });
    }

    /// 目的：验证周期源的**第一次** tick 立即完成（契约第 2 条）。
    ///
    /// 手段：构造运行时值，取一个 5 秒的周期，量测第一次 tick 的耗时。
    ///
    /// 判定：耗时远小于周期（这里取 `< 1 秒`）。若照搬 `async_io::Timer::interval`
    /// 的首次语义，本用例会等到 5 秒后才返回。
    #[test]
    fn interval_first_tick_is_immediate() {
        let rt = Runtime::<{ FULL }>::current();
        smol::block_on(async {
            let mut period = rt.interval(Duration::from_secs(5));
            let started = std::time::Instant::now();
            period.tick().await;
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "第一次 tick 应当立即完成"
            );
        });
    }

    /// 目的：验证自建周期源「锚定」而非「每响之后再等一个周期」。
    ///
    /// 手段：周期取 100ms；第一次 tick 立即，此后先故意耗掉 150ms 再第二次 tick，
    /// 量测第二次 tick 的耗时。
    ///
    /// 判定：如果实现是「上一觉之后再等 period」，第二次 tick 要再等 100ms
    /// （总 250ms）；锚定实现则在构造后 100ms 处就该到点，因此耗时应当**远小于
    /// 100ms**（这里取 `< 50ms`）。
    #[test]
    fn interval_is_anchored_to_its_creation_instant() {
        let rt = Runtime::<{ FULL }>::current();
        smol::block_on(async {
            let mut period = rt.interval(Duration::from_millis(100));
            period.tick().await; // 立即
            smol::Timer::after(Duration::from_millis(150)).await;
            let started = std::time::Instant::now();
            period.tick().await;
            assert!(
                started.elapsed() < Duration::from_millis(50),
                "锚定实现不该再等满一个周期"
            );
        });
    }

    /// 目的：验证「睡眠 future 是 `Send`」在**编译期可见**。
    ///
    /// 手段：编译期断言 `<Runtime<{FULL}> as TrDelay>::Delay: Send`。
    ///
    /// 判定：编译通过即为通过——把返回类型从 `impl Future`（RPITIT）换成
    /// **关联类型**之后，自动 trait 能出现在调用方的约束里。
    #[test]
    fn delay_future_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<<Runtime<{ FULL }> as TrDelay>::Delay>();
    }

    /// 目的：验证零周期在构造点被拒绝（契约第 5 条）。
    ///
    /// 手段：`#[should_panic]` 捕获值的 `interval(Duration::ZERO)`——async-io 自身
    /// **不**断言（会退化成忙循环），因此本后端必须自己挡。
    ///
    /// 判定：panic 文案含 "non-zero"。
    #[test]
    #[should_panic(expected = "non-zero")]
    fn interval_rejects_zero_period() {
        let rt = Runtime::<{ FULL }>::current();
        let _ = rt.interval(Duration::ZERO);
    }

    /// 目的：验证 `TrClock::Instant` 满足 trait 文档里的**结构约束**（编译期，
    /// 非空约束）。
    ///
    /// 手段：把 `<Runtime<{FULL}> as TrClock>::Instant` 传给一个形参带全部四条
    /// 约束的编译期断言函数。
    ///
    /// 判定：编译通过即为通过；若后端换成一个不满足约束的时刻类型，本测试无法编译。
    #[test]
    fn clock_instant_satisfies_the_structural_constraints() {
        fn assert_instant_<T>()
        where
            T: Copy + Ord + Add<Duration, Output = T> + Sub<T, Output = Duration> + 'static,
        {
        }
        assert_instant_::<<Runtime<{ FULL }> as TrClock>::Instant>();
    }

    /// 目的：验证 `now()` 与 `delay` **同源**——期限算术（`now() + Duration`）与
    /// 实耗（`now() − now()`）在同一条时间线上闭合。
    ///
    /// 手段：读一次 `now()` 记为起点，`delay(5ms)` 之后把「实耗」经
    /// `Sub<Self, Output = Duration>` 取出，再用 `Add<Duration>` 造一个未来期限
    /// 并与当前的 `now()` 比较。
    ///
    /// 判定：实耗 `>= 5ms`（睡眠真的按同一时钟计时），且 `now() + 10ms > now()`
    /// （加法给出的确实是未来）。若 `Instant` 与计时器不同源（例如一个是墙上时钟、
    /// 一个是别的基准），实耗会明显偏小或为负而判失败。
    #[test]
    fn clock_now_shares_the_time_base_with_delay() {
        let rt = Runtime::<{ FULL }>::current();
        let started = rt.now();
        smol::block_on(rt.delay(Duration::from_millis(5)));
        let elapsed: Duration = rt.now() - started;

        assert!(
            elapsed >= Duration::from_millis(5),
            "实耗 {elapsed:?} 小于睡眠时长：now() 与 delay 不同源"
        );
        let deadline = rt.now() + Duration::from_millis(10);
        assert!(deadline > rt.now(), "now() + Duration 必须是未来时刻");
    }

    /// 目的：验证 `TrClock` 门控在 `CLOCK` 上、`TrTime` 门控在 `DELAY + CLOCK` 上
    ///（「同时声明两位才有时刻与周期源」）。
    ///
    /// 手段：编译期断言同时声明 `DELAY` 与 `CLOCK` 两位的 `Runtime<{ DELAY | CLOCK }>`
    /// 实现了 `TrClock` 与 `TrTime`。
    ///
    /// 判定：编译通过即为通过——这是**正面**一半；反面一半（只声明 `DELAY` 时
    /// `now()` 不可用）由本模块文档的 `compile_fail` 用例钉住。若把门控错标在
    /// 别的位上，本测试无法编译。
    #[test]
    fn declared_delay_and_clock_caps_give_now_and_interval() {
        fn assert_clock_<T: TrClock>() {}
        fn assert_time_<T: TrTime>() {}
        assert_clock_::<Runtime<{ DELAY | CLOCK }>>();
        assert_time_::<Runtime<{ DELAY | CLOCK }>>();
    }
}
