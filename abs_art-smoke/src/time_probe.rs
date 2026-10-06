//! 跨后端 [`TrTime`] 行为契约的测试体（**泛型于 `R: TrTime`，收运行时值 `&R`**）。
//!
//! 与 `spawn_local` 那组同形：测试体只有**一份**，三个后端各拿它跑一遍，于是
//! 「同一份 trait、三个实现」最容易出的**语义漂移**变成可执行的判定。
//!
//! # 值化之后的形状变化（只改调用形状，不改判定）
//!
//! 最早的这些探针泛型于**类型** `T: TrTime`，调用点写 `<T as TrTime>::interval(p)`、
//! `<T as TrDelay>::delay(d)`——计时能力挂在类型上。值化之后能力挂到运行时**值**上，
//! 因此这里是 `rt.interval(p)` / `rt.delay(d)` / `rt.now()`，且多了一条类型级参数
//! 无法表达的新契约：[`probe_clock_is_monotonic_and_shares_delay_source`]。
//!
//! # 判定为什么用宽松上界
//!
//! 这里的用例要抓的是**结构性**偏差——「首次 tick 被推迟了一个周期」「锚定变成了
//! 每响之后再等一个周期」——而不是调度抖动。因此阈值一律留几百毫秒富余，宁可放过
//! 一次抖动，也不要变成一条随机红的用例。
//!
//! # 分工：时间来自**运行时值**，投递与驱动才来自作用域
//!
//! 本轮抽象层把本地队列放回一个独立的 [`TrLocalScope`] 值
//! （线程独占），而**计时与时刻留在运行时值上**：
//!
//! | 关切 | 宿主 | 本文件的探针 |
//! | --- | --- | --- |
//! | `delay` / `interval` / `now` / `timeout` | 运行时值 `R: TrTime` | 收 `&R`，调 `rt.delay(..)` / `rt.now()` |
//! | `spawn_local` / `run_until` / `block_on` | 作用域 `S: TrLocalScope` | 收 `&S`（`probe.rs` 那组） |
//!
//! 因此本文件的探针**不**改成收作用域：`TrLocalScope` 没有也不该有计时方法，
//! 需要等待的代码写 `R: TrTime` 并从运行时值上取用。
//!
//! 第 4 条（[`probe_clock_is_monotonic_and_shares_delay_source`]）与新增的第 5 条
//! （[`probe_time_capability_comes_from_the_runtime`]）是本分工的两面：前者判「同一个
//! 值的计时器与时刻同源」，后者判「计时能力挂在值上、而不是作用域上」。
//!
//! # 覆盖不到的部分
//!
//! - **零周期 panic**：`run_case` 在独立线程里跑，panic 会被记成「超时」而不是
//!   「如期 panic」，因此这条留在各后端的单测里（三个后端各有一条）；
//! - **落后时的行为**：跳过还是补齐属于各后端可自行选择的部分（见
//!   `abs_art::time` 的模块文档），因此这里的用例**不**在落后态下做判定。

use core::time::Duration;

use std::time::Instant;

// `rt.delay(..)` / `rt.now()` 这类调用靠**超 trait** 解析：`TrTime: TrDelay + TrClock`，
// 因此把 `R: TrTime` 写进探针的 where 子句、再把 `TrTime` 本身带进作用域就够了，
// 不必逐个导入那两个超 trait。
// `TrLocalScope` 只在第 5 条契约里出现：那里要**同时**收运行时值与作用域，用类型
// 参数的分工把「时间归值、投递归作用域」写死在签名上。
use abs_art::{TrInterval, TrLocalScope, TrTime};

/// 契约第 1 条：`delay` 不早于 `duration` 返回。
///
/// # Errors
///
/// `delay` 提前返回时给出实际耗时。
pub async fn probe_sleep_waits_at_least<R: TrTime>(rt: &R) -> Result<Duration, String> {
    const WAIT: Duration = Duration::from_millis(20);
    let started = Instant::now();
    rt.delay(WAIT).await;
    let elapsed = started.elapsed();
    if elapsed < WAIT {
        return Err(format!("delay({WAIT:?}) 只用了 {elapsed:?}：提前返回了"));
    }
    Ok(elapsed)
}

/// 契约第 1 条（补充）：`delay(0)` **立即**就绪。
///
/// 这不是细枝末节：「等到一个已经过去的时刻」在本家族里就是
/// `delay(deadline.saturating_duration_since(rt.now()))`，若 `delay(0)` 会阻塞，
/// 那个惯用法就不成立。因此把它钉成契约。
///
/// # Errors
///
/// `delay(0)` 耗时超过 100 ms（宽松上界，只抓结构性阻塞）时给出实际耗时。
pub async fn probe_zero_delay_completes_immediately<R: TrTime>(rt: &R) -> Result<Duration, String> {
    let started = Instant::now();
    rt.delay(Duration::ZERO).await;
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
pub async fn probe_interval_first_tick_is_immediate<R: TrTime>(rt: &R) -> Result<Duration, String> {
    // 周期取 5 秒：首次若被推迟，本用例会一直等到 5 秒（远超阈值）才返回。
    let mut period = rt.interval(Duration::from_secs(5));
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
pub async fn probe_interval_is_anchored<R: TrTime>(rt: &R) -> Result<Duration, String> {
    const PERIOD: Duration = Duration::from_millis(80);
    // 越过 1.5 个周期：锚定（或跳过错过的时刻）的实现在此刻已经到点，第二次 tick
    // 应当立即返回；「每响之后再等一个周期」的实现则要再等满 ~1 个周期。
    const OVERSHOOT: Duration = Duration::from_millis(120);
    // 阈值取 0.75 个周期：锚定 ≈ 0，跳过错过的时刻 ≈ 0.5 个周期，两者都过；
    // 「每响之后再等一个周期」≈ 1 个周期，判失败。
    const LIMIT: Duration = Duration::from_millis(60);

    let mut period = rt.interval(PERIOD);
    period.tick().await; // 立即
    rt.delay(OVERSHOOT).await;

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

/// 契约第 4 条（本版新增）：`now()` 单调不减，且与 `delay()` **同源**。
///
/// # 为什么这条契约是值化之后才能写的
///
/// 同源这件事在原设计里无法用类型表达：时刻是消费方配置项，计时器是后端类型，
/// 两者只能靠约定一致（tokio `start_paused` 下「睡在虚拟时钟、读在墙上时钟」的
/// 实测反例见 `dev-notes/runtime-tag-scope-clock-20261006-0923.md` §3.3）。
/// 本版 `TrTime: TrDelay + TrClock`，两者由**同一个运行时值**提供，因此这条
/// 契约可以被直接判定。
///
/// - **手段**：同一个运行时值上①连读三次 `now()`；②记下 `start = now()`，
///   `delay(30ms)`，再读 `after = now()`；
/// - **判定**：①三次读数单调不减；②`after − start >= 30ms`（用
///   `TrClock::Instant` 的 `Sub<Self, Output = Duration>` 结构约束折算）。
///   只要计时器与 `now()` 不同源（例如一个走虚拟时钟、一个走墙上时钟），第二条
///   要么前进量不足、要么根本不前进，判失败。
///
/// # Errors
///
/// `now()` 出现回退，或 `delay` 之后 `now()` 的前进量小于所睡的时长。
pub async fn probe_clock_is_monotonic_and_shares_delay_source<R: TrTime>(
    rt: &R,
) -> Result<Duration, String> {
    const WAIT: Duration = Duration::from_millis(30);

    // ① 单调不减：连续读三次，后一次不得早于前一次。
    // 注意 `TrClock::Instant` 只要求 `Copy + Ord`，不要求 `Debug`，因此这里不打印时刻值。
    let first = rt.now();
    let second = rt.now();
    let third = rt.now();
    if second < first {
        return Err("now() 出现回退：第二次读数早于第一次".to_owned());
    }
    if third < second {
        return Err("now() 出现回退：第三次读数早于第二次".to_owned());
    }

    // ② 同源：睡 WAIT 之后，now() 至少前进 WAIT。
    let start = rt.now();
    rt.delay(WAIT).await;
    let after = rt.now();
    let advanced = after - start;
    if advanced < WAIT {
        return Err(format!(
            "delay({WAIT:?}) 之后 now() 只前进了 {advanced:?}：时刻源与计时器不同源"
        ));
    }
    Ok(advanced)
}

/// 契约第 6 条（内层赢）：内层 future 先完成时 `timeout` 原样返回输出。
///
/// # Errors
///
/// 返回了 `Elapsed`（本不该超时）时给出提示。
pub async fn probe_timeout_inner_wins<R: TrTime>(rt: &R) -> Result<u8, String> {
    match rt.timeout(Duration::from_millis(50), async { 7u8 }).await {
        Ok(v) => Ok(v),
        Err(_) => Err("内层立刻完成，却报了超时".to_owned()),
    }
}

/// 契约第 6 条（期限先到）：内层挂起时 `timeout` 在期限附近返回 `Elapsed`。
///
/// # Errors
///
/// 未在宽松上界内返回、或错误文案不符时给出提示。
pub async fn probe_timeout_elapses<R: TrTime>(rt: &R) -> Result<Duration, String> {
    const LIMIT: Duration = Duration::from_millis(30);
    let started = Instant::now();
    let outcome = rt.timeout(LIMIT, core::future::pending::<u8>()).await;
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

/// 契约第 5 条（本轮新增）：**计时与时刻来自运行时值，而不是作用域**。
///
/// # 这条契约钉的是什么
///
/// 抽象层刚刚把本地队列放回独立的 [`TrLocalScope`] 值。若有人顺手把 `TrDelay` /
/// `TrClock` / `TrTime` 也实现到作用域上，「时间从哪来」就会再次变成两处来源
/// （值一处、作用域一处），值化好不容易建立的「同一个值提供计时器与时刻」立刻
/// 失效。本契约用**类型层面的证据**把这条分工冻住：
///
/// 1. [`time_owner::TimeOwner<S>`] 泛型于 `S: TrLocalScope`——**任何**作用域类型都能
///    代入，包括三个后端的 `LocalScope`；
/// 2. 本探针要求 `R: TrTime` 并**只**经 `&R` 执行 `delay`；
/// 3. 集成侧再以 [`time_owner::assert_time_owner_is_trtime`] 在编译期断言
///    `TimeOwner<Scope>: TrTime`——代入的正是「作用域」这个类型参数。
///
/// 第 3 条一旦成立就说明：拿到一个作用域，并不能让它的使用者获得 `TrTime`；
/// 能提供 `TrTime` 的是**另一个**值（这里由 `TimeOwner<S>` 代表运行时值）。
/// 反过来，如果哪天有人给作用域实现了 `TrTime`，那也拦不住——类型系统本来就不
/// 表达「负实现」。因此本契约是**回归闸门**：它把当前的边界写成了可执行的断言，
/// 边界被移动时至少会有人看见。
///
/// # 手段：作用域投递 + 驱动，运行时值计时
///
/// 探针同时收下运行时值 `rt: &R`（计时）与作用域 `scope: &S`（投递与驱动），
/// 两件事各归各的宿主：
///
/// 1. 用 `scope.spawn_local` 投递一个本地任务，任务体内**先**用运行时值的
///    `delay(2ms)` 睡眠、**再**向宿主回报——于是「谁在计时」在代码表面上就看得见；
/// 2. 用 `scope.run_until` 驱动作用域，等那个回报；
/// 3. 量墙上耗时。
///
/// 任务体里用的是 `rt.clone()` 出来的**自主运行时值**，而不是借来的 `&R`：投递到本地
/// 队列的 future 要求 `'static`，借用的 `&R` 活不过任务。这一步顺带说明了运行时值
/// 与作用域是两类可分离的东西——计时用的那份**值**被搬进了任务，作用域只负责投递与
/// 驱动。
///
/// # 判定
///
/// - 回报值必须是 `7`（本地任务确实被投递且被驱动）；
/// - 墙上耗时必须 `>= 2ms`（睡眠确实发生在运行时值的计时器上，而不是被跳过）。
///
/// # Errors
///
/// 本地任务没有回报（作用域没被驱动），或耗时小于所睡的时长（计时被跳过）。
pub async fn probe_time_capability_comes_from_the_runtime<R, S>(
    rt: &R,
    scope: &S,
) -> Result<Duration, String>
where
    R: TrTime + Clone + 'static,
    S: TrLocalScope,
{
    const WAIT: Duration = Duration::from_millis(2);
    const VALUE: u8 = 7;

    let (done_tx, done_rx) = async_channel::unbounded::<u8>();
    let started = Instant::now();

    // 投递走**作用域**，睡眠走**运行时值**：两个参数各司其职，签名本身即是分工。
    // 刻意丢掉句柄：本契约不关心任务结果经不经过句柄交回，只关心它被作用域驱动。
    let timer = rt.clone();
    let _handle = scope.spawn_local(async move {
        timer.delay(WAIT).await;
        let _ = done_tx.send(VALUE).await;
    });

    let got = scope
        .run_until(done_rx.recv())
        .await
        .map_err(|e| format!("本地任务没有回报（作用域未被驱动）：{e}"))?;
    let elapsed = started.elapsed();

    if got != VALUE {
        return Err(format!("本地任务回报 {got}，预期 {VALUE}"));
    }
    if elapsed < WAIT {
        return Err(format!(
            "delay({WAIT:?}) 之后墙上耗时只有 {elapsed:?}：睡眠被跳过了"
        ));
    }
    Ok(elapsed)
}

// 测试辅助模块：给第 5 条契约提供「一个**不是**作用域的计时宿主」。
//
// 本模块**不是**运行时的实现，而是契约的见证者：它证明 `TrDelay` / `TrClock` /
// `TrTime` 可以挂在一个与 `TrLocalScope` 完全无关的类型上——因此「时间来自哪」
// 是一个独立的选择，而不是本地队列的附带品。
//
// `TimeOwner<S>` 只把 `S` 当作类型参数（用 `PhantomData` 占位），不持有任何
// 作用域：它的计时能力与 `S` 的实现毫无关系。
pub mod time_owner {
    //! 第 5 条契约的见证类型：一个与作用域无关、却拥有 `TrTime` 的**值**。

    use core::{
        future::{Pending, pending},
        marker::PhantomData,
        ops::{Add, Sub},
        time::Duration,
    };

    use abs_art::{TrClock, TrDelay, TrInterval, TrLocalScope, TrTime};

    /// 只有计时能力、**没有**本地队列的运行时值形状。
    ///
    /// 类型参数 `S` 只用于说明「即使手上有一个 `S: TrLocalScope`，本值也与它无关」：
    /// 字段是 [`PhantomData`]，构造时不需要、也不会持有真正的队列。
    ///
    /// # Examples
    ///
    /// `TimeOwner` 自己实现 `TrTime`，因此可以直接调 `now`：
    ///
    /// ```
    /// use abs_art::TrClock;
    /// use abs_art_smoke::time_probe::time_owner::TimeOwner;
    ///
    /// let owner = TimeOwner::<()>::new();
    /// let _ = owner.now();
    /// ```
    pub struct TimeOwner<S> {
        /// 只占位：计时能力与 `S` 的实现无关，本值不持有任何作用域。
        _scope_: PhantomData<S>,
    }

    impl<S> TimeOwner<S> {
        /// 造一个见证值；不需要任何作用域（连 `S` 的实例都不需要）。
        ///
        /// `S` 可以写成 `()`，也可以写成某个真实作用域类型——两种写法都不影响它
        /// 的计时能力，这正是第 5 条契约要表达的独立性。
        pub fn new() -> Self {
            Self {
                _scope_: PhantomData,
            }
        }
    }

    impl<S> Default for TimeOwner<S> {
        fn default() -> Self {
            Self::new()
        }
    }

    /// 本见证值的时刻类型：与真实后端一样用墙上时钟（`std::time::Instant`）。
    ///
    /// 它满足 [`TrClock::Instant`] 的四个结构约束（`Copy + Ord`、
    /// `Add<Duration, Output = Self>`、`Sub<Self, Output = Duration>`、`'static`）。
    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    pub struct Instant(std::time::Instant);

    impl Add<Duration> for Instant {
        type Output = Instant;

        fn add(self, rhs: Duration) -> Instant {
            Instant(self.0 + rhs)
        }
    }

    impl Sub<Instant> for Instant {
        type Output = Duration;

        fn sub(self, rhs: Instant) -> Duration {
            self.0.saturating_duration_since(rhs.0)
        }
    }

    impl<S> TrDelay for TimeOwner<S> {
        /// 本见证值的睡眠 future。
        ///
        /// 刻意用**永不就绪**的 [`Pending`]：这里要钉的是「能力挂在哪个类型上」这个
        /// **类型层面**的事实，不是真实睡眠的时间量——真实睡眠由各后端在第 1~4 条
        /// 契约里实测（本文件上半部分），以及与本节配套的本地驱动用例。
        type Delay = Pending<()>;

        fn delay(&self, _duration: Duration) -> Self::Delay {
            pending()
        }
    }

    impl<S> TrClock for TimeOwner<S> {
        type Instant = Instant;

        fn now(&self) -> Self::Instant {
            Instant(std::time::Instant::now())
        }
    }

    impl<S> TrTime for TimeOwner<S> {
        /// 本见证值的周期源：由后端式的「每次 `tick` 都立刻就绪」的序列充当。
        ///
        /// 用 `core::future::Ready<()>` 而不是另写一个结构体：第 5 条契约不判周期
        /// 语义（那是第 2、3 条的事），只需要一个满足 `TrInterval` 的具体类型。
        type Interval = ReadyInterval;

        fn interval(&self, _period: Duration) -> Self::Interval {
            ReadyInterval
        }
    }

    /// 第 5 条契约用的最小周期源：每次 `tick` 都立刻就绪。
    #[derive(Debug)]
    pub struct ReadyInterval;

    impl TrInterval for ReadyInterval {
        type Tick<'a> = core::future::Ready<()>;

        fn tick(&mut self) -> Self::Tick<'_> {
            core::future::ready(())
        }
    }

    /// 编译期断言：**会话要求 `S: TrLocalScope` 时**，`TimeOwner<S>` 仍然是
    /// `TrTime`。
    ///
    /// 这条断言是第 5 条契约的一半证据：它把「计时能力」与「本地作用域」拆成两个
    /// 互不依赖的类型参数——`S` 只负责满足 `TrLocalScope`，`TimeOwner<S>` 负责提供
    /// `TrTime`。若哪天抽象层把计时搬到作用域上、并让 `TimeOwner` 失去 `TrTime`，
    /// 集成侧的这一处会**编译失败**。
    ///
    /// # Panics
    ///
    /// 本函数把断言写成 [`assert!`]：除类型检查外不做事。
    pub fn assert_time_owner_is_trtime<S>()
    where
        S: TrLocalScope,
        TimeOwner<S>: TrTime,
    {
        assert!(
            core::mem::size_of::<TimeOwner<S>>() == 0,
            "见证值只是类型参数占位，不应有运行时体积"
        );
    }
}
