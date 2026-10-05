//! `time`：计时能力——周期源（[`TrInterval`]）与超时（[`TrTime`]）。
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

use core::{
    future::Future,
    time::Duration,
};

use abs_art::{HasDelay, TrDelay, TrInterval, TrTime};

use crate::Runtime;

/// 本后端的周期源（[`TrTime::Interval`] 的具体类型）。
#[derive(Debug)]
pub struct Interval {
    inner_: compio::runtime::time::Interval,
}

impl TrInterval for Interval {
    /// compio 的 `Interval::tick` 是 `async fn`（不透明），用 ITIT 命名它。
    type Tick<'a> = impl Future<Output = ()> + 'a;

    fn tick(&mut self) -> Self::Tick<'_> {
        async move {
            // compio 的 `tick` 返回到期时刻（`Instant`）；本 crate 的契约不暴露
            // `Instant`（见 `abs_art::time` 模块文档），因此丢弃返回值。
            self.inner_.tick().await;
        }
    }
}

impl<const CAPS: usize> TrTime for Runtime<CAPS>
where
    [(); CAPS]: HasDelay,
{
    type Interval = Interval;

    fn interval(period: Duration) -> Self::Interval {
        Interval {
            inner_: compio::runtime::time::interval(period),
        }
    }
}

/// 让**本地作用域值**也承载一次性睡眠（[`TrDelay`]）与周期源（[`TrTime`]）。
///
/// 业务库手上只有作用域值（`S: TrLocalScope`），补上这两格之后写 `S: TrTime`
/// 一个约束就够，不必再引入第二个类型参数。compio 的本地队列归运行时所有，
/// 因此本实现**只借类型**、不读任何字段。
#[cfg(feature = "local_scope")]
impl TrDelay for crate::LocalScope {
    type Delay = impl Future<Output = ()>;

    fn delay(duration: Duration) -> Self::Delay {
        compio::runtime::time::sleep(duration)
    }
}

#[cfg(feature = "local_scope")]
impl TrTime for crate::LocalScope {
    type Interval = Interval;

    fn interval(period: Duration) -> Self::Interval {
        Interval {
            inner_: compio::runtime::time::interval(period),
        }
    }
}

#[cfg(test)]
mod tests {
    //! compio 后端的 `TrTime` 单测。
    //!
    //! 跨三后端的一致性契约由 `abs_art-smoke` 的 `time_contract` 用例负责；这里只
    //! 钉住「本后端能不能跑起来」这几条。

    use std::time::Instant;

    use super::*;

    /// 建一个 compio 运行时。
    fn rt_() -> compio::runtime::Runtime {
        compio::runtime::Runtime::new().expect("建 compio 运行时")
    }

    /// 目的：验证 `delay` 能真正睡到（compio 的 time 驱动被驱动）。
    ///
    /// 实施策略：在 compio 运行时里 await 1 ms 的 `delay`，量测实际耗时。
    ///
    /// 通过依据：正常返回且耗时 `>= 1 ms`；若 time 驱动没被驱动，本用例会挂死。
    #[test]
    fn delay_completes_and_waits_at_least_the_duration() {
        let rt = rt_();
        rt.block_on(async {
            let started = Instant::now();
            <Runtime<{ crate::FULL }> as TrDelay>::delay(Duration::from_millis(1)).await;
            assert!(
                started.elapsed() >= Duration::from_millis(1),
                "delay 不该提前返回"
            );
        });
    }

    /// 目的：验证周期源的**第一次** tick 立即完成（契约第 2 条）。
    ///
    /// 实施策略：取一个 5 秒的周期，量测第一次 tick 的耗时。
    ///
    /// 通过依据：耗时远小于周期（这里取 `< 1 秒`）——compio 的 `interval` 起点取
    /// 「现在」，因此首次 `tick` 立即就绪。
    #[test]
    fn interval_first_tick_is_immediate() {
        let rt = rt_();
        rt.block_on(async {
            let mut period =
                <Runtime<{ crate::FULL }> as TrTime>::interval(Duration::from_secs(5));
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
    /// 实施策略：`#[should_panic]` 捕获 `interval(Duration::ZERO)`——compio 自身
    /// 在 `interval_at` 上就会断言。
    ///
    /// 通过依据：panic 文案含 "zero"。
    #[test]
    #[should_panic(expected = "zero")]
    fn interval_rejects_zero_period() {
        let _ = <Runtime<{ crate::FULL }> as TrTime>::interval(Duration::ZERO);
    }
}
