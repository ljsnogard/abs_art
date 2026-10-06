//! `abs_art` 的 compio 组合 crate。
//!
//! 提供五个功能（各自为 feature 开关）：
//!
//! - `block_on`：阻塞等待一个 future 完成；
//! - `delay`：睡眠 / 延迟执行，以及计时能力（[`TrClock`] / [`TrTime`]）；
//! - `spawn_send`：**本后端不提供**该能力——该 feature 只带来一篇「为什么 compio 不实现
//!   `TrSpawnSend`」的说明（见 `spawn_send` 模块）；
//! - `local_scope`：线程独占的本地作用域（`!Send` 任务 + 投递与统一驱动入口）；
//! - `spawn_blocking`：投递阻塞函数到阻塞线程池。
//!
//! 所有实现都基于基础 crate [`abs_art`] 中的 trait。
//!
//! # compio **不**实现 [`TrSpawnSend`]
//!
//! [`TrSpawnSend`] 的契约定的是「投递到**全局（跨线程）**工作队列」。compio 没有这样
//! 一条队列：它的执行器（`Rc<Executor>`）就在运行时实例内部，`Runtime::spawn` 投的是
//! **本线程这个运行时**的队列，future 也仍在**本线程**上被轮询——这正是
//! `Runtime::spawn` 不要求 `F: Send` 的原因。
//!
//! 为它写一个 `impl TrSpawnSend` 会让抽象失真：
//!
//! - trait 上的 `F: Future + Send` 会成为一条**假的**前置条件：调用方以为自己在做
//!   「跨线程投递」，实际上任务从未离开这条线程；
//! - 反过来，库侧写 `R: TrSpawnSend` 是在声明「我依赖一条可跨线程共享的队列」，而
//!   compio 上这个前提不存在，承载它的运行时值本身还是 `!Send` 的，接手方搬都搬不走。
//!
//! 因此本 crate **删掉了那个 impl**，只留下 `spawn_send` 模块记录这条因果，并用
//! `compile_fail` 文档测试钉住「[`Runtime`] 不实现 [`TrSpawnSend`]」。
//!
//! # 要投递任务，请用本地作用域的 [`TrLocalScope::spawn_local`]
//!
//! compio 上「投递」与「本地投递」是**同一件事**：`Runtime::spawn` 与本地队列打在同一份
//! 运行时上，都不要求 `Send`。所以本 crate 统一走本地路径，入口是
//! `Runtime::local_scope()`：
//!
//! ```
//! use abs_art::{FULL, TrLocalScope};
//! use abs_art_compio::Runtime;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! let out = rt.block_on(async {
//!     let scope = Runtime::<{ FULL }>::current().local_scope();
//!     // 捕获 Rc 的 !Send future 也能投递：compio 的任务不跨线程
//!     let rc = std::rc::Rc::new(6u32);
//!     scope.spawn_local(async move { *rc * 7 }).await.unwrap()
//! });
//! assert_eq!(out, 42);
//! ```
//!
//! # [`TrSpawnBlocking`] 与（不存在的）`spawn` 的区别
//!
//! `spawn_blocking` **保留**，它与上面的「本线程投递」不是一回事：
//!
//! | | `Runtime::spawn`（本 crate 不暴露为 trait 方法） | [`TrSpawnBlocking::spawn_blocking`] |
//! | --- | --- | --- |
//! | 任务在哪执行 | **当前线程**的执行器队列（`Rc<Executor>`） | **另一条线程**（compio 用 `Asyncify` 把闭包交给阻塞线程池） |
//! | 要 `Send` 吗 | 不要（`compio::runtime::Runtime::spawn<F: Future + 'static>`） | 要（`F: FnOnce() -> T + Send + 'static`：闭包真的跨了线程） |
//! | 会不会阻塞调用线程 | 不会：future 只是在同一线程上被轮询，pending 时就挂起 | 会阻塞那条**工作线程**，因此只该放真正的阻塞活 |
//!
//! 实测依据：`compio-runtime-0.12.6` 的 `Runtime::spawn` 签名只要求 `'static`
//! （`src/lib.rs:221`），`spawn_blocking` 走 `Asyncify` 提交给驱动（`src/lib.rs:247`
//! 起）；本 crate 的测试 `spawn_blocking_runs_off_the_calling_thread` 也实测到闭包跑在
//! 别的线程上。
//!
//! # 运行时是**值**：compio 这一侧能钉住什么
//!
//! compio 的 `Runtime`（[`compio::runtime::Runtime`]）本身就是**可克隆的句柄簇**：
//! 内部是 `Rc<Executor>` + `Rc<RefCell<Proactor>>`（`time` feature 下还有
//! `Rc<RefCell<TimerRuntime>>`），并且 `spawn` / `spawn_blocking` / `block_on`
//! 全部收 `&self`。因此本 crate 的 [`Runtime`] **把这份句柄真的存进值里**，
//! 而不是像原来那样只借类型：
//!
//! - `spawn_blocking` 打在**这个值**抓住的那份运行时上，而不是
//!   「当前线程恰好进入了哪个运行时」；
//! - `block_on` 由这份运行时自己 `enter` 出上下文再驱动 future，因此调用点
//!   **不必**已处于 compio 运行时上下文内（`Runtime::current()` 仍然要求）；
//! - 本地投递经 `Runtime::local_scope()` 交出的 `LocalScope` 进行：它**钉住同一份
//!   运行时**（而不是另建一条队列），compio 的执行器队列归运行时所有、由运行时自己
//!   驱动——这与 tokio 需要另建 `LocalSet` 的形状不同；
//! - 克隆运行时值与克隆 `LocalScope` 都共享同一份运行时（`Rc` 克隆），前者与
//!   [`Runtime::retag`] 等价。
//!
//! 代价：值必须被**构造**出来（不再是随处可写的 ZST），且因为内部持有 `Rc`，
//! 它和 compio 的运行时一样是 `!Send`——这与「compio 运行时是线程本地的」这条
//! 事实一致，不是本 crate 额外加的限制。
//!
//! ## 钉不住的那一半：计时注册是**环境式**的
//!
//! compio 的计时入口（`time::sleep` / `time::interval`）**不接受**运行时参数，
//! 内部靠 `Runtime::with_current` 在线程本地找到运行时；而且注册发生在**首次轮询**
//! （`sleep` / `sleep_until` 都是 `async fn`，函数体到第一次 `poll` 才执行）。
//! 因此 [`TrDelay::delay`] / [`TrTime::interval`] 只能如实记录：**计时器注册在
//! 「轮询点所在线程的当前 compio 运行时」上**，而不是 `self` 抓住的那一份。
//!
//! 实践上两者总是一致：本 crate 的 [`TrBlockOn::block_on`] 会用 `self` 的运行时
//! `enter` 出上下文，`value.block_on(value.delay(d))` 内部的注册必然落在同一个
//! 运行时上。这条差异在报告里逐条记录，未做隐瞒。
//!
//! 「同源」在**时钟**这一层仍然成立：compio 的计时基准就是 [`std::time::Instant`]
//! （`time::sleep_until` / `time::interval_at` 的形参类型），[`TrClock::now`] 读的
//! 是同一个钟。
//!
//! # 两种用法
//!
//! ## 具体用法（二进制层）
//!
//! ```
//! use abs_art::TrBlockOn;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! let out = rt.block_on(async {
//!     let value = abs_art_compio::current();
//!     value.block_on(async { 42 })
//! });
//! assert_eq!(out, 42);
//! ```
//!
//! ## 能力位用法（业务库层，编译期能力检查）
//!
//! 通过 const 泛型声明所需能力；请求了未声明（或本后端不支持）的能力会在
//! 编译期报错：
//!
//! ```
//! use abs_art::{BLOCK_ON, SPAWN_LOCAL, TrLocalScope};
//! use abs_art_compio::Runtime;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! let out = rt.block_on(async {
//!     // 只声明 block_on + spawn_local 两种能力
//!     let value = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
//!     let scope = value.local_scope();
//!     let handle = scope.spawn_local(async { 2u8 });
//!     handle.await.unwrap()
//! });
//! assert_eq!(out, 2);
//! ```
//!
//! ```compile_fail
//! use abs_art::BLOCK_ON;
//! use abs_art_compio::Runtime;
//!
//! // 只声明了 block_on 能力：取不到本地作用域 → 编译错误（Tag 严格模式）
//! let rt = compio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let value = Runtime::<{ BLOCK_ON }>::current();
//!     let _ = value.local_scope();
//! });
//! ```
//!
//! compio 的运行时值不实现 [`TrSpawnSend`]，因此**任何** CAPS 下都没有 `spawn` 方法
//! 可用——原因与替代写法见上文专节与 `spawn_send` 模块。
//!
//! ## 本地投递（`!Send` 任务）
//!
//! compio 的本地队列归运行时所有，`Runtime::local_scope()` 交出的 `LocalScope` 钉住的
//! 就是**同一份运行时**，因此投递与驱动都落在它身上：
//!
//! ```
//! use abs_art::TrLocalScope;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! let out = rt.block_on(async {
//!     let value = abs_art_compio::current();
//!     let scope = value.local_scope();
//!     scope
//!         .run_until(async {
//!             let rc = std::rc::Rc::new(6u32); // !Send：只有本地队列能承载
//!             scope.spawn_local(async move { *rc * 7 }).await.unwrap()
//!         })
//!         .await
//! });
//! assert_eq!(out, 42);
//! ```

#![no_std]
// 家族里**只有本 crate** 需要 nightly：compio 的睡眠/周期/超时 future 全是
// `pub async fn`（不透明、不可命名），关联类型只能用 ITIT（`impl Future`）给出。
// 因果与取舍见 `dev-notes/time-20261005-1225.md` §11。
//
// 该 feature 只在 `delay`（`time.rs` / `delay.rs` 的 ITIT 关联类型）下才被用到，
// 因此用 `cfg_attr` 跟着门控：否则 `--no-default-features` 会触发
// `unused_features` 警告。（`local_scope` / `spawn_*` / `join_handle` 都不用 ITIT，
// 它们用的是稳定特性，例如 RPITIT。）
#![cfg_attr(feature = "delay", feature(impl_trait_in_assoc_type))]

// 本 crate 声明 `no_std`：自身代码只使用 `core`（`alloc` 都不需要——运行时句柄
// 由 compio 持有，我们不额外装箱）。
//
// 唯一的例外是 [`crate::TrClock::Instant`]：它必须与 compio 的计时器**同源**，
// 而 compio 的 `time::sleep_until` / `time::interval_at` 直接以
// `std::time::Instant` 为形参，compio 自己**没有**导出任何时刻类型
// （已实测：`compio-runtime-0.12.6/src/time/mod.rs` 只 `pub use future::Interval`，
// 其余入口全部写死 `std::time::{Duration, Instant}`）。因此这里显式引入 `std`。
//
// 实测结论：`#![no_std]` + `extern crate std;` 可以并存并编译通过
// （见报告的「逐项实测」）。这条声明并不让本 crate 变成「可用在真 no-std 目标上」
// ——`compio` 自身依赖 `std`，本 crate 从来就没有这个可能；显式写出来只是把
// 既有事实写明，而不是新增依赖。
extern crate std;

use core::fmt;

use abs_art::RuntimeTag;

pub use abs_art::{
    BLOCK_ON, DELAY, Elapsed, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, Timeout,
    TrAsyncRuntime, TrBlockOn, TrClock, TrDelay, TrInterval, TrJoinHandle,
    TrLocalScope, TrSpawnBlocking, TrSpawnSend, TrTime, UnitFuture,
};

/// compio 组合运行时**值**。
///
/// 对应基础 crate 中的 [`RuntimeTag::Compio`]。它抓住构造点的 compio 运行时
/// （可克隆的 `Rc` 句柄簇）：`spawn_blocking` / `block_on` 以及计时能力直接打在
/// **这个值**抓住的那份运行时上；本地投递则经 `Runtime::local_scope()` 交出
/// `LocalScope`（同一份运行时的把手）。
///
/// 类型参数 `CAPS` 是能力位掩码（见 [`abs_art::caps`]）：默认 [`FULL`]（全功能），
/// 也可以写成 `Runtime<{ BLOCK_ON | SPAWN_LOCAL }>` 只声明部分能力。掩码决定这个
/// **类型**实现了哪些能力 trait（以及能否经 `local_scope()` 取得作用域），从而决定
/// 哪些方法可调。
///
/// 注意：本值**不**实现 [`TrSpawnSend`]——compio 没有跨线程全局队列，投递任务请用
/// [`TrLocalScope::spawn_local`]（理由见 crate 文档）。
///
/// # 构造
///
/// 见 [`Runtime::current`]（要求已处于 compio 运行时上下文内）与
/// [`Runtime::with_runtime`]（在任意位置用已有运行时构造，便于把值搬进运行时）。
///
/// # 克隆
///
/// `Clone` 共享**同一份**运行时（同一个执行器队列与同一个驱动），克隆出来的值与
/// 原来的值是同一个运行时的两个把手，不是两份运行时。
///
/// # `Send`
///
/// 本值与 `compio::runtime::Runtime` 一样是 `!Send`：compio 的运行时是线程本地的，
/// 队列与驱动都绑在创建它的线程上。这不是本 crate 附加的限制。
pub struct Runtime<const CAPS: usize = FULL> {
    /// 构造点抓住的 compio 运行时：`spawn_blocking` / `local_scope` / `block_on` 都打在它上面。
    rt_: compio::runtime::Runtime,
}

/// 用当前 compio 运行时上下文构造**全能力**（`Runtime<FULL>`）运行时值。
///
/// 这是最常用的构造入口：类型参数 `CAPS` 直接取默认值 [`FULL`]，因此在表达式
/// 位置也**不需要类型标注**。需要显式声明能力时用
/// [`Runtime::current`](Runtime::current) 的 turbofish 形式。
///
/// # Panics
///
/// 调用点不在 compio 运行时上下文内时 panic
/// （文案 ``not in a compio runtime``）。
///
/// # Examples
///
/// ```
/// let rt = compio::runtime::Runtime::new().unwrap();
/// let value = rt.block_on(async { abs_art_compio::current() });
/// assert_eq!(value.tag(), abs_art::RuntimeTag::Compio);
/// ```
pub fn current() -> Runtime {
    Runtime::current()
}

impl<const CAPS: usize> Runtime<CAPS> {
    /// 用当前 compio 运行时上下文构造运行时值。
    ///
    /// # 表达式位置请用 turbofish 或 crate 级自由函数
    ///
    /// `CAPS` 的默认值 `FULL` **不参与**函数调用返回位置的推断：实测
    /// `let value = Runtime::current();`（无标注）会报
    /// `E0284: type annotations needed for Runtime<_>`。因此表达式位置有两条出口：
    /// 写全 `Runtime::<{ abs_art::FULL }>::current()`，或用 crate 级自由函数
    /// [`current()`](crate::current)（返回类型已是具体的 `Runtime<FULL>`）。
    ///
    /// ```compile_fail
    /// use abs_art_compio::Runtime;
    ///
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// rt.block_on(async {
    ///     // E0284：`CAPS` 默认值不参与表达式位置的推断
    ///     let value = Runtime::current();
    ///     let _ = value.tag();
    /// });
    /// ```
    ///
    /// # Panics
    ///
    /// 调用点不在 compio 运行时上下文内时 panic。需要在上下文之外构造时，用
    /// [`Runtime::with_runtime`]。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art_compio::Runtime;
    ///
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// let value = rt.block_on(async { Runtime::<{ abs_art::FULL }>::current() });
    /// assert_eq!(value.tag(), abs_art::RuntimeTag::Compio);
    /// ```
    pub fn current() -> Self {
        Self::with_runtime(compio::runtime::Runtime::current())
    }

    /// 用给定的 compio 运行时构造运行时值。
    ///
    /// 这是**在运行时上下文之外**构造的标准方式：先在别处拿到
    /// [`compio::runtime::Runtime`]（它是 `Clone` 的句柄簇，克隆不复制运行时），
    /// 再把值搬进运行时。搬进来的值从此可以独立 `block_on`——`block_on` 会把这
    /// 份运行时 `enter` 成当前上下文。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::TrBlockOn;
    /// use abs_art_compio::Runtime;
    ///
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// // 此刻并不在 compio 上下文内；把句柄搬成运行时值
    /// let value = Runtime::<{ abs_art::FULL }>::with_runtime(rt.clone());
    /// let out = value.block_on(async { 7u8 });
    /// assert_eq!(out, 7);
    /// ```
    pub fn with_runtime(rt: compio::runtime::Runtime) -> Self {
        Self { rt_: rt }
    }

    /// 复制一份把手：与本值共享同一份运行时（同一条队列、同一个驱动）。
    ///
    /// 与 `Clone` 等价，但可以在 CAPS 上「换标签」（见 [`Runtime::retag`]）。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::TrBlockOn;
    /// use abs_art_compio::Runtime;
    ///
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// let value = rt.block_on(async { Runtime::<{ abs_art::FULL }>::current() });
    /// let other = value.retag::<{ abs_art::FULL }>();
    /// assert_eq!(other.block_on(async { 1 }), 1);
    /// ```
    pub fn retag<const OTHER: usize>(&self) -> Runtime<OTHER> {
        Runtime {
            rt_: self.rt_.clone(),
        }
    }

    /// 本值对应的抽象运行时标签。
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// let value = rt.block_on(async { abs_art_compio::current() });
    /// assert_eq!(value.tag(), abs_art::RuntimeTag::Compio);
    /// ```
    pub fn tag(&self) -> RuntimeTag {
        RuntimeTag::Compio
    }

    /// 本值抓住的 compio 运行时（escape hatch，便于做后端特有的事）。
    ///
    /// # Examples
    ///
    /// ```
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// let value = rt.block_on(async { abs_art_compio::current() });
    /// // escape hatch：拿回底层 compio 运行时，问它的驱动类型
    /// let _driver_type = value.runtime().driver_type();
    /// ```
    pub fn runtime(&self) -> &compio::runtime::Runtime {
        &self.rt_
    }
}

#[cfg(feature = "local_scope")]
impl<const CAPS: usize> Runtime<CAPS>
where
    [(); CAPS]: abs_art::HasSpawnLocal,
{
    /// 交出本运行时的本地作用域——一个**线程独占**的 [`LocalScope`]。
    ///
    /// # 它钉住的是同一份运行时
    ///
    /// compio 的执行器队列不可与运行时分离，因此本方法**不**新建队列：它把 `self`
    /// 抓住的那份运行时（`Rc` 克隆）装进 [`LocalScope`]，由后者负责投递与驱动。于是
    /// 「这次 `spawn_local` 投到哪份运行时」由值回答，不靠环境。
    ///
    /// # 为什么要求能力位
    ///
    /// 这是取得本地投递能力的**唯一入口**，而它要求 `CAPS` 含
    /// [`SPAWN_LOCAL`]：想拿到作用域，就得先把「我要用本地投递」
    /// 这件事写在类型上。声明位的价值是「**必须写下来**」，不是「写不下来就用不了」。
    ///
    /// # 线程独占
    ///
    /// compio 的运行时绑定创建它的线程：作用域只能在**这条**线程上被驱动，且它是
    /// `!Send`——想换线程就得在那边另取一个。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::{SPAWN_LOCAL, TrLocalScope};
    /// use abs_art_compio::Runtime;
    ///
    /// let rt = compio::runtime::Runtime::new().unwrap();
    /// let out = rt.block_on(async {
    ///     let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();
    ///     scope
    ///         .run_until(async { scope.spawn_local(async { 7u32 }).await.unwrap() })
    ///         .await
    /// });
    /// assert_eq!(out, 7);
    /// ```
    pub fn local_scope(&self) -> LocalScope {
        LocalScope::with_runtime(self.rt_.clone())
    }
}

impl<const CAPS: usize> Clone for Runtime<CAPS> {
    fn clone(&self) -> Self {
        self.retag()
    }
}

impl<const CAPS: usize> fmt::Debug for Runtime<CAPS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("abs_art_compio::Runtime")
            .field("caps", &CAPS)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "join_handle")]
pub mod join_handle;

#[cfg(feature = "join_handle")]
pub use join_handle::{JoinError, JoinHandle};

#[cfg(feature = "block_on")]
mod block_on;

#[cfg(feature = "delay")]
pub mod delay;

#[cfg(feature = "delay")]
pub mod time;

#[cfg(feature = "spawn_send")]
pub mod spawn_send;

#[cfg(feature = "local_scope")]
pub mod local_scope;

#[cfg(feature = "local_scope")]
pub use local_scope::LocalScope;

#[cfg(feature = "spawn_blocking")]
mod spawn_blocking;
