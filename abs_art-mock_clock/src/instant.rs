//! 手动时钟的时刻类型：把「毫秒刻度」映射到具体类型（可扩展）。

use core::ops::{Add, Sub};
use core::time::Duration;

/// 手动时钟的时刻类型。
///
/// 这是本 crate 的**扩展点**之一：内置实现是 [`MillisInstant`]；第三方时刻类型
/// （例如 `embedded-timers` 的 `Instant64<FREQ>`）只要满足这里的三条要求，就可以
/// 让手动时钟跑在自己的时刻类型上。
///
/// # 约束来自哪里
///
/// 前四条与 [`abs_art::TrClock::Instant`] 的结构约束一致（`Copy + Ord + Add + Sub`），
/// 因为装饰器会把本类型直接当成 `TrClock::Instant` 报出去；`from_millis` / `as_millis`
/// 是手动时钟内部的刻度换算。
///
/// # Examples
///
/// ```
/// use core::time::Duration;
/// use abs_art_mock_clock::{MillisInstant, MockInstant};
///
/// let a = MillisInstant::from_millis(1_000);
/// let b = a + Duration::from_millis(500);
/// assert_eq!(b - a, Duration::from_millis(500));
/// assert_eq!(b.as_millis(), 1_500);
/// ```
pub trait MockInstant:
    Copy + Ord + Add<Duration, Output = Self> + Sub<Self, Output = Duration> + 'static
{
    /// 从毫秒刻度构造时刻。
    fn from_millis(millis: u64) -> Self;

    /// 折算成毫秒刻度。
    fn as_millis(self) -> u64;
}

/// 内置时刻类型：毫秒计数器。
///
/// 手动时钟内部就以毫秒为刻度，因此本类型是零成本的；小于 1ms 的推进会被截断到
/// 0ms（需要更细刻度时实现自己的 [`MockInstant`]）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MillisInstant(u64);

impl MillisInstant {
    /// 零时刻。
    pub const ZERO: Self = Self(0);

    /// 从毫秒数构造。
    #[must_use]
    pub const fn new(millis: u64) -> Self {
        Self(millis)
    }

    /// 取毫秒数。
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Add<Duration> for MillisInstant {
    type Output = Self;

    fn add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(duration.as_millis() as u64))
    }
}

impl Sub<Self> for MillisInstant {
    type Output = Duration;

    /// 饱和相减：`earlier - later` 得 `Duration::ZERO`，不 panic。
    fn sub(self, other: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(other.0))
    }
}

impl MockInstant for MillisInstant {
    fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    fn as_millis(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    //! [`MillisInstant`] 的加减与刻度换算。

    use core::time::Duration;

    use super::{MillisInstant, MockInstant};

    /// 目的：验证时刻加减与毫秒换算是自洽的。
    ///
    /// 手段：用 `from_millis` / `Add<Duration>` / `Sub<Self>` / `as_millis` 走一遍。
    ///
    /// 判断：加法后毫秒数为两者之和，减法取回原 `Duration`。
    #[test]
    fn arithmetic_round_trips() {
        let start = MillisInstant::from_millis(1_000);
        let later = start + Duration::from_millis(250);
        assert_eq!(later.as_millis(), 1_250);
        assert_eq!(later - start, Duration::from_millis(250));
    }

    /// 目的：验证「早减晚」饱和到 0 而不是 panic 或回绕。
    ///
    /// 手段：用较小时刻减去较大时刻。
    ///
    /// 判断：结果为 `Duration::ZERO`。
    #[test]
    fn subtraction_saturates() {
        let early = MillisInstant::new(10);
        let late = MillisInstant::new(99);
        assert_eq!(early - late, Duration::ZERO);
    }

    /// 目的：验证亚毫秒推进被截断（本类型的既定精度）。
    ///
    /// 手段：推进 999µs。
    ///
    /// 判断：毫秒数不变。
    #[test]
    fn sub_millisecond_advance_truncates() {
        let start = MillisInstant::ZERO;
        let later = start + Duration::from_micros(999);
        assert_eq!(later.as_millis(), 0);
    }
}
