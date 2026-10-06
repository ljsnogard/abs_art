//! `delay`：睡眠与延迟执行。
//!
//! [`TrDelay`] 是**值方法**（收 `&self`）：能力挂在运行时值上。但 compio 的计时
//! 入口是**环境式**的——`time::sleep` 不接受运行时参数，内部靠
//! `Runtime::with_current` 在线程本地注册计时器，且注册发生在**首次轮询**。因此
//! 本实现如实记录：计时器落在「轮询点所在线程的当前 compio 运行时」上，见
//! [crate 文档](crate)的「钉不住的那一半」一节。

use core::{future::Future, time::Duration};

use compio::runtime::Runtime as CompioRuntime;

use abs_art::{HasDelay, TrDelay};

use crate::{CompioCaps_, Runtime, join_handle::JoinHandle};

/// 异步地睡眠 `duration`。
///
/// 自由函数形态：不做值化，直接走 compio 的**环境式**入口（当前线程的 compio
/// 运行时）。要绑定到某个具体的运行时值，用 [`TrDelay::delay`]。
pub async fn sleep(duration: Duration) {
    compio::runtime::time::sleep(duration).await
}

/// 在 `interval` 之后执行 `f`（本地版），返回 [`JoinHandle`]。
///
/// `f` 会被投递到当前线程的 compio 运行时上运行。compio 的运行时是线程本地的，
/// 因此这里**不要求** `Send`——与需要真正跨线程的
/// [`TrSpawnBlocking::spawn_blocking`](abs_art::TrSpawnBlocking::spawn_blocking) 形成
/// 对照：后者声明 `F: Send + 'static`（闭包被交给阻塞线程池），而本函数只声明「投到当前
/// 运行时」。这正是 compio 上 `spawn` 的唯一形态，因此本 crate **不**为它实现
/// `TrSpawnSend`（理由见 crate 文档）；需要本地投递时用
/// [`TrLocalScope::spawn_local`](abs_art::TrLocalScope::spawn_local)。
pub fn delayed<X, F>(interval: Duration, f: F) -> JoinHandle<X>
where
    X: 'static,
    F: FnOnce() -> X + 'static,
{
    CompioRuntime::with_current(|rt| {
        rt.spawn(async move {
            compio::runtime::time::sleep(interval).await;
            f()
        })
    })
    .into()
}

impl<const CAPS: usize> TrDelay for Runtime<CAPS>
where
    [(); CAPS]: HasDelay,
    [(); CAPS]: CompioCaps_,
{
    /// 本后端的睡眠 future 类型。
    ///
    /// compio 的 `sleep` 是 `pub async fn`（返回**不透明**类型），家族里只有本 crate
    /// 需要 `impl_trait_in_assoc_type`（ITIT）把它命名出来；因果见
    /// `dev-notes/time-20261005-1225.md` §11。它按 compio 的设计是 `!Send`
    /// （compio 用 `assert_not_impl!(TimerFuture, Send)` 钉住），但 trait 只要求
    /// `Future<Output = ()>`，因此不构成障碍。
    type Delay = impl Future<Output = ()>;

    /// 返回一个等待 `duration` 之后完成的未来。
    ///
    /// 计时源来自 compio 的**线程本地**计时器注册（见[模块文档](self)）；
    /// [`TrClock::now`](abs_art::TrClock::now) 与它读的是同一个钟。
    fn delay(&self, duration: Duration) -> Self::Delay {
        compio::runtime::time::sleep(duration)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `delay` 功能单元测试。

    use core::time::Duration;

    use abs_art::TrDelay;

    use super::*;

    /// 目的：验证自由函数 `sleep` 在 compio 运行时内可以正常完成（time driver
    /// 被驱动）。
    ///
    /// 实施策略：创建 compio 运行时，在 `rt.block_on` 中 await 一个 1ms 的 `sleep`。
    ///
    /// 通过依据：`sleep` 正常返回且没有 panic；若 time driver 未被驱动，future
    /// 将永远 pending，`rt.block_on` 无法返回（测试挂死）。
    #[test]
    fn sleep_completes() {
        let rt = CompioRuntime::new().unwrap();

        rt.block_on(async { sleep(Duration::from_millis(1)).await });
    }

    /// 目的：验证 `delayed` 在指定间隔之后执行闭包并返回其结果。
    ///
    /// 实施策略：在 `rt.block_on` 中调用 `delayed`，await 其 JoinHandle 取回
    /// 闭包的返回值。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn delayed_runs_after_interval() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async { delayed(Duration::from_millis(1), || 6 * 7).await.unwrap() });

        assert_eq!(out, 42);
    }

    /// 目的：验证值方法 `TrDelay::delay` 在运行时值上可用，且至少睡满给定时长
    /// ——即值化之后计时能力确实没有退化（走的是同一条 compio 计时路径）。
    ///
    /// 实施策略：在 compio 运行时上下文内构造运行时值，量测
    /// `value.delay(Duration::from_millis(1))` 的实际耗时。
    ///
    /// 通过依据：正常返回且耗时 `>= 1ms`；若计时器没被驱动，本用例会挂死。
    #[test]
    fn value_delay_waits_at_least_the_duration() {
        let rt = CompioRuntime::new().unwrap();

        rt.block_on(async {
            let value = crate::current();
            let started = std::time::Instant::now();
            value.delay(Duration::from_millis(1)).await;
            assert!(
                started.elapsed() >= Duration::from_millis(1),
                "delay 不该提前返回"
            );
        });
    }
}
