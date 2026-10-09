//! `abs_art` 的 tokio 组合 crate。
//!
//! 提供五个功能（各自为 feature 开关）：
//!
//! - `block_on`：阻塞等待一个 future 完成（**不**驱动本地队列）；
//! - `delay`：睡眠 / 延迟执行，以及计时能力（[`TrClock`] / [`TrTime`]）；
//! - `spawn_send`：投递任务到全局工作队列；
//! - `local_scope`：**本线程那条本地队列**的别名（`!Send` 任务 + 统一驱动入口）；
//! - `spawn_blocking`：投递阻塞函数到阻塞线程池。
//!
//! 所有实现都基于基础 crate [`abs_art`] 中的 trait。
//!
//! # 运行时是**值**，本地队列是**线程本地的别名**
//!
//! [`Runtime`] 是一个真正的运行时值：它抓住调用点的 tokio `Handle`。于是
//! `spawn` / `spawn_blocking` 打在**这个值**持有的运行时上（不是「当前线程恰好
//! 在哪个运行时里」）。
//!
//! 本地队列**不在**值里面：它在本线程的 `thread_local!` 里，[`LocalScope`] 只是
//! 那条队列的一个别名（`Clone` 即多一个别名、同一线程上 `local_scope()` 幂等）。
//! 于是 `spawn_local` / `run_until` 打的是**本线程**那条队列。
//!
//! 同一个进程里存在两套运行时（例如测试二进制里 tokio 与 compio 并存）时，
//! 「哪个运行时」由你手上的值回答；「哪条本地队列」则由**线程**回答。
//!
//! 代价：值必须被**构造**出来（不像原来的 ZST 那样随处可写），且本地队列的寿命
//! 等于线程寿命——线程退出时未完成的本地任务随之消失。
//!
//! # 两种用法
//!
//! ## 具体用法（二进制层）
//!
//! ```
//! use abs_art::TrBlockOn;
//! use abs_art_tokio::Runtime;
//!
//! let rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let tokio_rt = abs_art_tokio::current();
//!     tokio_rt.block_on(async { 42 })
//! });
//! ```
//!
//! ## 能力位用法（业务库层，编译期能力检查）
//!
//! 通过 const 泛型声明所需能力；请求了未声明（或本后端不支持）的能力会在
//! 编译期报错：
//!
//! ```
//! use abs_art::{BLOCK_ON, SPAWN_SEND, TrBlockOn, TrSpawnSend};
//! use abs_art_tokio::Runtime;
//!
//! // 只声明 block_on + spawn_send 两种能力
//! let tokio_rt = tokio::runtime::Runtime::new().unwrap();
//! tokio_rt.block_on(async {
//!     let rt = Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current();
//!     let _ = rt.spawn(async { 2 });
//!     rt.block_on(async { 1 })
//! });
//! ```
//!
//! ```compile_fail
//! use abs_art::{BLOCK_ON, TrSpawnSend};
//! use abs_art_tokio::Runtime;
//!
//! // 只声明了 block_on 能力，spawn（spawn_send）不可用 → 编译错误（Tag 严格模式）
//! let rt = tokio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let rt = Runtime::<{ BLOCK_ON }>::current();
//!     let _ = rt.spawn(async { 1 });
//! });
//! ```
//!
//! ## 本地投递（`!Send` 任务）
//!
//! 本地队列是一个独立的作用域值，由运行时值交出来：
//!
//! ```
//! use abs_art::TrLocalScope;
//!
//! let rt = tokio::runtime::Runtime::new().unwrap();
//! let out = rt.block_on(async {
//!     let scope = abs_art_tokio::current().local_scope(); // 要求 CAPS 含 SPAWN_LOCAL
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

extern crate alloc;

// 本地队列存放在本线程的 `thread_local!` 里（`local_scope` feature），测试也需要
// `std::thread`。tokio 本身是 std crate，因此这里**不新增**任何依赖，只是把既有的
// std 依赖显式写出来。两个条件用 `any` 合并，避免同时成立时重复声明 `std`。
#[cfg(any(feature = "local_scope", test))]
extern crate std;

use core::fmt;

use abs_art::RuntimeTag;

pub use abs_art::{
    BLOCK_ON, DELAY, Elapsed, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, Timeout, TrAsyncRuntime,
    TrBlockOn, TrClock, TrDelay, TrInterval, TrJoinHandle, TrLocalScope, TrSpawnBlocking,
    TrSpawnSend, TrTime, UnitFuture,
};

/// 本后端的**完整能力集**。
///
/// tokio 有真正的跨线程全局工作队列（`spawn` 投到 `Handle` 的全局队列、
/// `spawn_blocking` 投到阻塞线程池），五种能力一个不缺，因此本常量与
/// [`abs_art::FULL`] **数值相同**。与之相对，`abs_art-compio` 的 `FULL` 不含
/// `SPAWN_SEND`（它没有那种队列）——所以「完整」是**按后端**回答的问题，本常量是
/// 本后端的答案。
///
/// 与 [`abs_art::FULL`] 的分工：
///
/// - [`abs_art::FULL`] 是**位集合意义上**的「全部能力位」（基础 crate 给出的全集）；
/// - 本常量是**本后端实现得了的**那一部分。
///
/// 两者当前相等只是「tokio 每种能力都有」的结果，不是同义反复；某个后端补齐不了
/// 某个位时，差异体现在各后端的 `FULL` 上。本 crate 的 [`Runtime`] 默认类型参数
/// 用的就是本常量。
pub const FULL: usize = abs_art::FULL;

/// tokio 组合运行时**值**。
///
/// 对应基础 crate 中的 [`RuntimeTag::Tokio`]。它抓住构造点的 tokio 句柄，
/// 是 `Send + Sync` 的把手；本地队列由 [`LocalScope`] 承载（见类型与 crate 文档）。
///
/// 类型参数 `CAPS` 是能力位掩码（见 [`abs_art::caps`]）：默认 [`FULL`]（全功能），
/// 也可以写成 `Runtime<{ BLOCK_ON | SPAWN_SEND }>` 只声明部分能力。掩码决定这个
/// **类型**实现了哪些能力 trait，从而决定哪些方法可调。
///
/// # 构造
///
/// 见 [`Runtime::current`]（要求已处于 tokio 运行时上下文）与
/// [`Runtime::with_handle`]（在任意位置用已有句柄构造，便于把值搬进运行时）。
///
/// # 克隆
///
/// `Clone` 复制的是运行时**把手**（同一个 tokio 运行时）——不是新运行时。
/// 本地队列不在本类型里，因此克隆不涉及队列（见 [`LocalScope`]）。
pub struct Runtime<const CAPS: usize = FULL> {
    /// 构造点抓住的 tokio 句柄：`spawn` / `spawn_blocking` / `block_on` 都打在它上面。
    ///
    /// 本值是 `Send + Sync` 的——**本地队列不在它里面**（那是线程独占资源，
    /// 见 [`crate::LocalScope`]）。
    handle_: tokio::runtime::Handle,
}

/// 用当前 tokio 运行时上下文构造**全能力**（`Runtime<FULL>`）运行时值。
///
/// 这是最常用的构造入口：类型参数 `CAPS` 直接取默认值 [`FULL`]，因此在表达式
/// 位置也**不需要类型标注**。需要显式声明能力时用
/// [`Runtime::current`](Runtime::current) 的 turbofish 形式。
///
/// # Panics
///
/// 调用点不在 tokio 运行时上下文内时 panic（`Handle::current()` 的行为）。
///
/// # Examples
///
/// ```
/// let rt = tokio::runtime::Runtime::new().unwrap();
/// let value = rt.block_on(async { abs_art_tokio::current() });
/// assert_eq!(value.tag(), abs_art::RuntimeTag::Tokio);
/// ```
pub fn current() -> Runtime {
    Runtime::current()
}

/// 当前 tokio 运行时上下文里的运行时值；**不在**上下文内时返回 `Option::None`。
///
/// 与 [`current`] 的唯一区别是**不 panic**：它把「调用点是否处于 tokio 运行时
/// 上下文内」变成可查询的返回值，供调用方（例如 `smux_v1` 的 `CurrentConnCfg`）在
/// debug 构建下给出自己的提示，而不是等到 `Handle::current()` 抛 panic 才发现。
///
/// # Examples
///
/// ```
/// let rt = tokio::runtime::Runtime::new().unwrap();
/// assert!(rt.block_on(async { abs_art_tokio::try_current().is_some() }));
/// // 上下文之外（普通线程）得到 `None`，不 panic：
/// let outside = std::thread::spawn(abs_art_tokio::try_current).join().unwrap();
/// assert!(outside.is_none());
/// ```
pub fn try_current() -> Option<Runtime> {
    Runtime::<{ FULL }>::try_current()
}

impl<const CAPS: usize> Runtime<CAPS> {
    /// 用当前 tokio 运行时上下文构造运行时值。
    ///
    /// # Panics
    ///
    /// 调用点不在 tokio 运行时上下文内时 panic（`Handle::current()` 的行为）。
    /// 需要在上下文之外构造时，用 [`Runtime::with_handle`]。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art_tokio::Runtime;
    ///
    /// let rt = tokio::runtime::Runtime::new().unwrap();
    /// let value = rt.block_on(async { abs_art_tokio::current() });
    /// assert_eq!(value.tag(), abs_art::RuntimeTag::Tokio);
    /// ```
    pub fn current() -> Self {
        Self::with_handle(tokio::runtime::Handle::current())
    }

    /// 当前上下文里的运行时值；**不在**上下文内时返回 `Option::None`（不 panic）。
    ///
    /// 这是 [`Runtime::current`] 的可查询版本（`tokio::runtime::Handle::try_current`
    /// 的薄封装）：「调用点是否处于 tokio 运行时上下文内」在这里是一个返回值，
    /// 而不是一次 panic。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art_tokio::Runtime;
    ///
    /// let rt = tokio::runtime::Runtime::new().unwrap();
    /// let inside = rt.block_on(async { Runtime::<{ abs_art::FULL }>::try_current() });
    /// assert!(inside.is_some());
    /// let outside = std::thread::spawn(Runtime::<{ abs_art::FULL }>::try_current)
    ///     .join()
    ///     .unwrap();
    /// assert!(outside.is_none());
    /// ```
    pub fn try_current() -> Option<Self> {
        tokio::runtime::Handle::try_current().ok().map(Self::with_handle)
    }

    /// 用给定的 tokio 句柄构造运行时值。
    ///
    /// 这是**在运行时上下文之外**构造的标准方式：先在别处取到 `Handle`，
    /// 再把值搬进运行时（或搬给别的线程——见类型文档的 `Send` 说明）。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::TrBlockOn;
    /// use abs_art_tokio::Runtime;
    ///
    /// let rt = tokio::runtime::Runtime::new().unwrap();
    /// let handle = rt.handle().clone();
    /// let value = Runtime::<{ abs_art::FULL }>::with_handle(handle);
    /// let out = rt.block_on(async { value.block_on(async { 7u8 }) });
    /// assert_eq!(out, 7);
    /// ```
    pub fn with_handle(handle: tokio::runtime::Handle) -> Self {
        Self { handle_: handle }
    }

    /// 复制一份把手：与本值共享同一个运行时句柄与同一条本地队列。
    ///
    /// 与 `Clone` 等价，但可以在 CAPS 上「换标签」（见 [`Runtime::retag`]）。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::TrBlockOn;
    /// use abs_art_tokio::Runtime;
    ///
    /// let rt = tokio::runtime::Runtime::new().unwrap();
    /// rt.block_on(async {
    ///     let a = Runtime::<{ abs_art::FULL }>::current();
    ///     let b = a.retag::<{ abs_art::FULL }>();
    ///     assert_eq!(b.block_on(async { 1 }), 1);
    /// });
    /// ```
    pub fn retag<const OTHER: usize>(&self) -> Runtime<OTHER> {
        Runtime {
            handle_: self.handle_.clone(),
        }
    }

    /// 本值对应的抽象运行时标签。
    pub fn tag(&self) -> RuntimeTag {
        RuntimeTag::Tokio
    }

    /// 本值抓住的 tokio 句柄（escape hatch，便于做后端特有的事）。
    pub fn handle(&self) -> &tokio::runtime::Handle {
        &self.handle_
    }
}

#[cfg(feature = "local_scope")]
impl<const CAPS: usize> Runtime<CAPS>
where
    [(); CAPS]: abs_art::HasSpawnLocal,
{
    /// 交出本线程的本地队列——它的一个**别名**（[`LocalScope`]）。
    ///
    /// # 为什么要求能力位
    ///
    /// 这是取得本地投递能力的**唯一入口**，而它要求 `CAPS` 含
    /// [`SPAWN_LOCAL`]：想拿到作用域，就得先把「我要用本地
    /// 投递」这件事写在类型上。声明位的价值是「**必须写下来**」，不是「写不下来就用不了」。
    ///
    /// # 幂等：同一线程上永远给同一条队列
    ///
    /// 队列在**本线程的 `thread_local!`** 里，本方法只是取出它的 `Rc` 别名。因此
    /// 同一线程上调用多次得到的是**同一条** `LocalSet`，与克隆作用域等价；
    /// 「同一线程多条 `LocalSet`」这套 tokio 用法被**显式排除**（家族只对齐 compio
    /// 的语义，见 [`TrLocalScope`]）。
    ///
    /// 另外，本方法**不需要**已进入 tokio 上下文：队列的创建与句柄无关（用
    /// `Runtime::with_handle` 造的值也能交出作用域）。真正需要上下文的是驱动它的
    /// [`run_until`](abs_art::TrLocalScope::run_until)。
    ///
    /// # 线程独占
    ///
    /// tokio 的 `LocalSet` 绑定本条线程：作用域只能在**这条**线程上被驱动，
    /// 且它是 `!Send`——想换线程就得在那边另取一个。队列不随运行时把手跨线程。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::{SPAWN_LOCAL, TrLocalScope};
    /// use abs_art_tokio::Runtime;
    ///
    /// let rt = tokio::runtime::Runtime::new().unwrap();
    /// let out = rt.block_on(async {
    ///     let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();
    ///     scope
    ///         .run_until(async { scope.spawn_local(async { 7u32 }).await.unwrap() })
    ///         .await
    /// });
    /// assert_eq!(out, 7);
    /// ```
    pub fn local_scope(&self) -> LocalScope {
        LocalScope::with_handle(self.handle_.clone())
    }
}

impl<const CAPS: usize> Clone for Runtime<CAPS> {
    fn clone(&self) -> Self {
        self.retag()
    }
}

impl<const CAPS: usize> fmt::Debug for Runtime<CAPS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("abs_art_tokio::Runtime")
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
mod spawn_send;

#[cfg(feature = "local_scope")]
pub mod local_scope;

#[cfg(feature = "local_scope")]
pub use local_scope::LocalScope;

#[cfg(feature = "spawn_blocking")]
mod spawn_blocking;

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端**完整能力集常量**的单元测试。

    use abs_art::{HasBlockOn, HasClock, HasDelay, HasSpawnBlocking, HasSpawnLocal, HasSpawnSend};

    use crate::FULL;

    /// 目的：验证本后端的 `FULL` 是**本后端的完整能力集**——数值与
    /// [`abs_art::FULL`] 相同，且确实覆盖全部六种能力。
    ///
    /// 手段：运行期断言 `FULL == abs_art::FULL` 且 `FULL` 含 `CLOCK`
    /// （`FULL & CLOCK != 0`）；编译期把 `[(); FULL]` 依次传给六个标记 trait 的断言函数。
    ///
    /// 判定：断言成立且编译通过即为通过。tokio 有真正的跨线程全局工作队列与阻塞
    /// 线程池，五种能力一个不缺，因此这里与基础 crate 的全集相等；若将来某位不再
    /// 支持，本用例会先失败，迫使各后端 `FULL` 与文档同步。
    #[test]
    fn full_is_this_backend_complete_capability_set() {
        fn assert_block_on<T: HasBlockOn>() {}
        fn assert_delay<T: HasDelay>() {}
        fn assert_spawn_send<T: HasSpawnSend>() {}
        fn assert_spawn_local<T: HasSpawnLocal>() {}
        fn assert_spawn_blocking<T: HasSpawnBlocking>() {}
        fn assert_clock<T: HasClock>() {}

        assert_eq!(
            FULL,
            abs_art::FULL,
            "tokio 的完整能力集应与基础 crate 的全集同值"
        );
        assert_ne!(FULL & abs_art::CLOCK, 0, "本后端的完整能力集必须含 CLOCK");

        assert_block_on::<[(); FULL]>();
        assert_delay::<[(); FULL]>();
        assert_spawn_send::<[(); FULL]>();
        assert_spawn_local::<[(); FULL]>();
        assert_spawn_blocking::<[(); FULL]>();
        assert_clock::<[(); FULL]>();
    }
}
