//! 时间能力：**异步睡眠**与**周期唤醒**。
//!
//! 本模块只定义 trait 与一个零依赖的错误类型，**不含任何实现**——具体实现由组合
//! crate（tokio / compio / smol）用各自的运行时提供，与 `block_on` / `delay` /
//! `spawn_*` 的「声明在 `abs_art`、实现在后端」是同一套分工。
//!
//! # 与 [`TrDelay`](crate::TrDelay) 的关系
//!
//! [`TrDelay`](crate::TrDelay) 是**最小原语**（睡一段），本模块是它上面那层
//! **可用形状**：周期源与超时。两者的能力位共用 [`DELAY`](crate::DELAY)：
//! 后者只是「本后端具备计时能力」的声明位，而 `delay` 与 `interval` 在各后端
//! 来自**同一个**运行时 feature（tokio 的 `time`、compio 的 `time`、smol 的
//! `Timer`），没有拆成两位的必要。
//!
//! # 为什么这里**没有** `sleep_until` / `interval_at` / `timeout_at`
//!
//! 这三个都要求「**绝对时刻**」，也就是要求一个 `Instant` 类型。而 `abs_art` 是
//! `no_std`、零依赖的基础抽象 crate：它既不能写 `std::time::Instant`，也不该为此
//! 引入一个外部的时钟抽象 crate（那会让该 crate 变成 `abs_art` 的公开依赖）。
//!
//! 因此本模块把这个决定留给**持有自己的时钟**的消费方，而它们本来就需要一个
//! 时钟（连接级 epoch、每子流「最后活动」毫秒等记账都要它）。三者都是**可推导**的：
//!
//! ```text
//! sleep_until(t)  ≡  sleep(t − clock.now())
//! timeout_at(t)   ≡  TrTime::timeout(t − clock.now(), future)
//! interval_at(s)  ≡  sleep(s − clock.now()) 之后接 interval(period)
//! ```
//!
//! 好处有两个：`abs_art` 不必再养一套与 `embedded-timers` 之类重复的时钟抽象；
//! 而**消费方可以注入假时钟**——这正是「超时与宽限期可确定性验收」的前提。
//!
//! # 语义契约（三个后端必须一致）
//!
//! 「同一份 trait、三个实现」最容易出的问题是**语义漂移**。因此下列几条是
//! **契约**而不是建议，跨后端的一致性由 `abs_art-smoke` 的 `time_contract`
//! 用例钉住：
//!
//! 1. [`TrDelay::delay`] 至少在 `duration` 之后才完成；且 `duration` 为零时**立即**
//!    就绪——后者是「等到一个已经过去的时刻」= `delay(0)` 这个惯用法成立的前提，
//!    消费方折算绝对期限时依赖它；
//! 2. [`TrTime::interval`] 的**第一次** [`TrInterval::tick`] **立即**完成
//!    （起点 = 调用 `interval` 的那一刻）；
//! 3. 其后每次 `tick` **相位对齐**到起点（即 `起点 + k × period`），而不是
//!    「上一次 tick 之后再等一个 period」；
//! 4. 某一轮落后时**跳过**已错过的时刻（不连续补齐）——与 compio 的相位对齐
//!    语义一致；
//! 5. `period` 为零时 [`TrTime::interval`] **panic**；
//! 6. [`TrTime::timeout`] 在同一轮里「内层 future 就绪」与「期限到期」**都**成立时，
//!    **内层赢**（先轮询内层）。

use crate::TrDelay;

use core::{
    fmt,
    future::Future,
    marker::PhantomPinned,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

/// 后端的**计时能力**：按周期唤醒，以及带超时地等一个 future。
///
/// 它是 [`TrDelay`](crate::TrDelay) 的**超 trait**：**睡眠不再另起名字**——一次性的
/// 「睡一段」就是 [`TrDelay::delay`]，本 trait 不重复提供 `sleep`（那会是同一个能力的
/// 第二个名字）。于是能力分两层：
///
/// | 层 | 提供什么 |
/// | --- | --- |
/// | [`TrDelay`](crate::TrDelay) | 一次性睡眠 `delay` |
/// | `TrTime` | 周期源 [`TrTime::interval`] + 组合出的 [`TrTime::timeout`] |
///
/// 两个方法都是**关联函数**（不收 `self`）：后端的计时源来自运行时的线程本地上下文，
/// 不需要由值携带，因此业务库只需把**类型**穿进来，不必先拿到某个计时器值。
///
/// # Examples
///
/// ```
/// use core::time::Duration;
///
/// use abs_art::{TrDelay, TrInterval, TrTime};
///
/// /// 业务代码只需泛型于 `D: TrTime`，具体后端由最终二进制选中。
/// async fn tick_twice<D: TrTime>() {
///     D::delay(Duration::from_millis(1)).await;
///     let mut period = D::interval(Duration::from_millis(1));
///     period.tick().await;
///     period.tick().await;
/// }
/// ```
pub trait TrTime: TrDelay {
    /// 周期源的**具体类型**（由后端给出）。
    ///
    /// 是关联类型而不是 `impl TrInterval`：调用方因此能命名它（存进结构体、
    /// 对它写自动 trait 约束）。
    type Interval: TrInterval;

    /// 以 `period` 为周期产生唤醒的周期源。
    ///
    /// 首次 [`TrInterval::tick`] 立即完成，其后相位对齐到本调用发生的时刻；
    /// 完整契约见[模块文档](self)。
    ///
    /// # Panics
    ///
    /// `period` 为零时 panic（三个后端一致；零周期的「无穷序列」没有意义）。
    ///
    /// # Examples
    ///
    /// 见 [`TrTime`]。
    fn interval(period: Duration) -> Self::Interval;

    /// 要求 `future` 在 `duration` 之内完成。
    ///
    /// 内层先完成则原样返回其输出；期限先到则返回 [`Elapsed`]，并**丢弃**内层 future
    /// （取消就是丢弃，与三个后端的其余等待者一致）。
    ///
    /// # 为什么是 trait 的**默认实现**，返回 [`Timeout`] 这个**具体类型**
    ///
    /// 它只依赖 [`TrDelay::delay`]，因此一份实现就够——少三处漂移的机会。返回类型是
    /// 本 crate 的**具体结构体**（不是 `impl Future`、也不是关联类型），于是：一份实现、
    /// 可命名（能存进结构体、能写 `where …: Send`）、零装箱零分配。
    ///
    /// **后端不应覆盖它**：三后端语义一致靠的正是「这一份实现 + `abs_art-smoke`
    /// 的契约矩阵」。
    ///
    /// # Panics
    ///
    /// 本方法自身不 panic。
    ///
    /// # Examples
    ///
    /// ```
    /// use core::time::Duration;
    ///
    /// use abs_art::{Elapsed, TrTime};
    ///
    /// async fn demo<D: TrTime>() -> Result<u8, Elapsed> {
    ///     D::timeout(Duration::from_millis(10), async { 7u8 }).await
    /// }
    /// ```
    fn timeout<F>(duration: Duration, future: F) -> Timeout<Self, F>
    where
        F: Future,
        Self: Sized,
    {
        Timeout::new(duration, future)
    }
}

/// [`TrTime::interval`] 的返回值：按周期产生唤醒的**序列**。
///
/// 之所以是独立 trait 而不是某个具体类型：三个后端的周期源类型互不相同
/// （`tokio::time::Interval` / `compio` 的 `Interval` / 自建），而它们都是外部
/// 类型，孤儿规则不允许直接为它们实现本 crate 的 trait。
///
/// 本 trait **不**返回 tick 对应的 `Instant`——理由见[模块文档](self)的
/// 「为什么这里没有 `sleep_until` …」。
pub trait TrInterval {
    /// `tick(&mut self)` 返回的 future——它是**真正带生命周期参数的 GAT**：
    /// 三个后端的 `tick` 都是 `async fn`（返回不透明类型），只有借用 `self` 的
    /// 关联类型才能把它命名出来。
    ///
    /// 命名出来之后，调用方可以把它存进结构体、也可以对它写自动 trait 约束。
    type Tick<'a>: Future<Output = ()>
    where
        Self: 'a;

    /// 等到周期里的下一个时刻。
    ///
    /// # Examples
    ///
    /// 见 [`TrTime`]。
    fn tick(&mut self) -> Self::Tick<'_>;
}

/// [`TrTime::timeout`] 的失败：期限已到，而内层 future 还没完成。
///
/// 字段是私有的：本类型只能由 [`TrTime::timeout`] 产生，调用方无法伪造「期限已到」
/// 再把它经 `?` 塞进自己的错误类型里，从而不会出现「没超时却报超时」的假信号。
///
/// # Examples
///
/// ```
/// use core::time::Duration;
///
/// use abs_art::{Elapsed, TrTime};
///
/// async fn demo<D: TrTime>() -> Result<u8, Elapsed> {
///     D::timeout(Duration::from_millis(10), async { 7u8 }).await
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed(());

impl fmt::Display for Elapsed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("期限已到")
    }
}

impl core::error::Error for Elapsed {}

/// [`TrTime::timeout`] 的返回值：把「后端的一次性睡眠」与「调用方的 future」
/// 并排驱动。
///
/// # 为什么是本 crate 的**具体类型**，而不是 `impl Future` 或关联类型
///
/// - **可命名**：`Timeout<D, F>` 能写进结构体、能出现在 `where …: Send` 里；
///   它的自动 trait 由字段**结构地**决定（`Send` 当且仅当 `D::Delay: Send`
///   且 `F: Send`），于是「这个超时 future 能不能跨线程投递」在编译期就可见。
/// - **一份实现**：三个后端共用它，不必各写一遍（少三处漂移）。
/// - **零装箱、零分配**：两个字段都按值持有。
///
/// # 结构性 pin
///
/// 结构体带 [`PhantomPinned`]，因此它**永远不是 `Unpin`**；`poll` 里用
/// `get_unchecked_mut` + `Pin::new_unchecked` 把两个字段各自当作 pinned——见
/// [`Timeout::poll`] 的 SAFETY 注释。这是本 crate 唯一一处 `unsafe`。
#[derive(Debug)]
pub struct Timeout<D, F>
where
    D: TrDelay,
    F: Future,
{
    delay_: D::Delay,
    future_: F,
    /// 让本类型**永远** `!Unpin`：结构性 pin 的正确性前提（见 `poll` 的 SAFETY）。
    _pin_: PhantomPinned,
}

impl<D, F> Timeout<D, F>
where
    D: TrDelay,
    F: Future,
{
    /// 起一个「`duration` 到点，或 `future` 先完成」的竞争。
    ///
    /// 计时器在**此刻**就起（而不是等到第一次 `poll`），因此期限从调用点算起。
    ///
    /// # Examples
    ///
    /// ```
    /// use core::time::Duration;
    ///
    /// use abs_art::{Elapsed, TrTime};
    ///
    /// async fn demo<D: TrTime>() -> Result<u8, Elapsed> {
    ///     D::timeout(Duration::from_millis(10), async { 7u8 }).await
    /// }
    /// ```
    pub fn new(duration: Duration, future: F) -> Self {
        Self {
            delay_: D::delay(duration),
            future_: future,
            _pin_: PhantomPinned,
        }
    }
}

impl<D, F> Future for Timeout<D, F>
where
    D: TrDelay,
    F: Future,
{
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY（结构性 pin，`futures::future::Select` / `tokio::time::Timeout` 同款）：
        //
        // 1. `Timeout` 含 `PhantomPinned`，**永远不实现 `Unpin`**，因此一旦被 pin
        //    就不再可能被安全地移动（`Pin::get_mut` 拿不到 `&mut Self`）；
        // 2. `poll` 也**没有**实现 `Drop`：不存在「析构后字段被移动」的路径；
        // 3. 因此 `delay_` / `future_` 的地址在 pin 之后保持不变，把它们各自视为
        //    pinned 是成立的。两个字段都只在 `poll` 里被 `Pin<&mut _>` 使用，
        //    不做任何移动。
        let this = unsafe { self.get_unchecked_mut() };
        let delay = unsafe { Pin::new_unchecked(&mut this.delay_) };
        let future = unsafe { Pin::new_unchecked(&mut this.future_) };

        // 先问内层：同一轮里两边都就绪时**内层赢**（契约第 6 条）。
        if let Poll::Ready(output) = future.poll(cx) {
            return Poll::Ready(Ok(output));
        }
        if delay.poll(cx).is_ready() {
            return Poll::Ready(Err(Elapsed(())));
        }
        Poll::Pending
    }
}

/// 把「完成时返回某个值」的 future 适配成 `Output = ()`。
///
/// 供后端把自家睡眠 future 接上 [`TrDelay`]（例如 smol 的 `Timer` 完成时返回到期
/// 时刻，而 [`TrDelay::Delay`] 要求 `Output = ()`）。
///
/// 只在 `F: Unpin` 时实现 [`Future`]——因此这里的 pin 投影是**安全**的，不需要
/// `unsafe`（本 crate 的唯一一处 `unsafe` 在 [`Timeout`] 里，那里 `F` 由调用方给出、
/// 不能假定 `Unpin`）。
#[derive(Debug)]
pub struct UnitFuture<F>(F);

impl<F> UnitFuture<F> {
    /// 包住 `future`。
    pub fn new(future: F) -> Self {
        Self(future)
    }
}

impl<F> Future for UnitFuture<F>
where
    F: Future + Unpin,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // `F: Unpin` ⇒ `Self: Unpin` ⇒ 这里可以安全地拿到 `&mut F`。
        Pin::new(&mut self.get_mut().0).poll(cx).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    //! `TrTime` 语义契约的**后端无关**部分。
    //!
    //! 这里用一个「虚拟时间」的 [`TrTime`] 实现，使 `timeout` 的判定完全确定性；
    //! 三个**真实后端**的一致性由 `abs_art-smoke` 的 `time_contract` 用例负责。

    use core::{
        future::pending,
        sync::atomic::{AtomicU64, Ordering},
    };
    // 本 crate 是 `no_std`，`core` 的 prelude 里没有 `ToString`；测试下经
    // `#[cfg(test)] extern crate std;` 取它，免得为了断言文案引入 `alloc`。
    use std::string::ToString;

    use super::*;

    /// 虚拟时间的计时能力：`delay(d)` 把虚拟时钟推进 `d` 后**立刻**就绪。
    ///
    /// 它证明一件事：本模块的抽象是**可注入**的——消费方能用假时钟做确定性验收，
    /// 而不必依赖真实运行时的调度。
    struct FakeTime_;

    /// 虚拟时钟（毫秒）。
    static VIRTUAL_MILLIS: AtomicU64 = AtomicU64::new(0);

    impl TrDelay for FakeTime_ {
        type Delay = FakeDelay_;

        fn delay(duration: Duration) -> Self::Delay {
            FakeDelay_ {
                duration_: duration,
                done_: false,
            }
        }
    }

    /// 虚拟时间的睡眠 future：**第一次被 `poll` 时**把虚拟时钟推进 `duration`，
    /// 随即就绪（且只推进一次）。
    ///
    /// 刻意**不**在 `delay(..)` 被调用时推进：真实后端在那一刻只是把期限记下来
    /// （tokio 记在 `Sleep` 里、compio 在首次轮询时才算），到点才醒。若在构造时就
    /// 推进，「内层先赢」的用例会因为 [`Timeout`] 构造即起计时器而看到时钟前进。
    struct FakeDelay_ {
        duration_: Duration,
        done_: bool,
    }

    impl Future for FakeDelay_ {
        type Output = ();

        fn poll(self: core::pin::Pin<&mut Self>, _cx: &mut core::task::Context<'_>) -> Poll<()> {
            let this = self.get_mut();
            if !this.done_ {
                this.done_ = true;
                VIRTUAL_MILLIS.fetch_add(this.duration_.as_millis() as u64, Ordering::SeqCst);
            }
            Poll::Ready(())
        }
    }

    impl TrTime for FakeTime_ {
        type Interval = FakeInterval_;

        fn interval(period: Duration) -> Self::Interval {
            assert!(period > Duration::ZERO, "`period` must be non-zero.");
            FakeInterval_(period)
        }
    }

    /// 虚拟时间的周期源：每次 `tick` 推进一个周期。
    struct FakeInterval_(Duration);

    impl TrInterval for FakeInterval_ {
        /// 虚拟时间的 tick future：推进虚拟时钟后立刻就绪。
        type Tick<'a> = core::future::Ready<()>;

        fn tick(&mut self) -> Self::Tick<'_> {
            VIRTUAL_MILLIS.fetch_add(self.0.as_millis() as u64, Ordering::SeqCst);
            core::future::ready(())
        }
    }

    /// 目的：验证内层 future 先完成时 `timeout` 原样返回输出。
    ///
    /// 实施策略：用虚拟时间后端 `FakeTime_` 包一个立刻就绪的 future。
    ///
    /// 通过依据：结果为 `Ok(7)`。
    #[test]
    fn timeout_returns_output_when_inner_wins() {
        let out = block_on_(FakeTime_::timeout(
            Duration::from_millis(10),
            async { 7u8 },
        ));
        assert_eq!(out, Ok(7u8));
    }

    /// 目的：验证期限先到时 `timeout` 返回 `Elapsed` 且文案固定。
    ///
    /// 实施策略：内层用永不就绪的 future，期限 5 ms。
    ///
    /// 通过依据：结果为 `Err`，且 `Display` 为「期限已到」。
    #[test]
    fn timeout_elapses_on_a_pending_inner_future() {
        let out = block_on_(FakeTime_::timeout(
            Duration::from_millis(5),
            pending::<u8>(),
        ));
        let elapsed = out.expect_err("期限已到应当是错误");
        assert_eq!(elapsed.to_string(), "期限已到");
    }

    /// 目的：验证同期限的两个 `timeout` 各自独立，且虚拟时钟按 `sleep` 累加。
    ///
    /// 实施策略：从 0 起连续两次 3 ms 的 `timeout`（内层挂起），读虚拟时钟。
    ///
    /// 通过依据：虚拟时钟恰好推进 6 ms——说明 `sleep` 是唯一的时间来源，
    /// 不存在额外的隐藏等待。
    #[test]
    fn virtual_clock_only_advances_by_sleep() {
        VIRTUAL_MILLIS.store(0, Ordering::SeqCst);
        let _ = block_on_(FakeTime_::timeout(
            Duration::from_millis(3),
            pending::<u8>(),
        ));
        let _ = block_on_(FakeTime_::timeout(
            Duration::from_millis(3),
            pending::<u8>(),
        ));
        assert_eq!(VIRTUAL_MILLIS.load(Ordering::SeqCst), 6);
    }

    /// 目的：验证零周期在 `interval` 构造点被拒绝。
    ///
    /// 实施策略：`#[should_panic]` 捕获 `FakeTime_::interval(Duration::ZERO)`。
    ///
    /// 通过依据：panic 文案含 "must be non-zero"。
    #[test]
    #[should_panic(expected = "must be non-zero")]
    fn interval_rejects_zero_period() {
        let _ = <FakeTime_ as TrTime>::interval(Duration::ZERO);
    }

    /// 目的：验证 `Elapsed` 满足错误类型的三件套（`Debug` / `Display` / `Error`）。
    ///
    /// 实施策略：编译期断言 `Elapsed: core::error::Error`，并检查 `source` 为 `None`。
    ///
    /// 通过依据：编译通过，且 `source()` 返回 `None`。
    #[test]
    fn elapsed_is_a_well_formed_error() {
        fn assert_error_<E: core::error::Error>() {}
        assert_error_::<Elapsed>();

        let elapsed = block_on_(FakeTime_::timeout(
            Duration::from_millis(1),
            pending::<u8>(),
        ))
        .expect_err("期限已到应当是错误");
        assert!(core::error::Error::source(&elapsed).is_none());
    }

    /// 极简的 `block_on`：本 crate 是 `no_std` + 零依赖，测试里不引入运行时，
    /// 因此手动把一个 future 抽干（`FakeTime_` 的所有等待都立刻就绪，不会挂起）。
    ///
    /// 轮数有上限：真出现「假后端居然挂起」的回归时，应当**立刻失败**而不是死循环。
    fn block_on_<F: Future>(future: F) -> F::Output {
        const MAX_ROUNDS: usize = 64;
        let mut future = core::pin::pin!(future);
        // 不需要被唤醒：`FakeTime_` 的每一次等待都立即就绪，轮询即可推进。
        let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
        for _ in 0..MAX_ROUNDS {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
        panic!("虚拟时间的后端不该挂起：{MAX_ROUNDS} 轮仍未就绪");
    }
}
