//! `abs_art` 的 smol 组合 crate。
//!
//! 提供五个功能（各自为 feature 开关）：
//!
//! - `block_on`：阻塞等待一个 future 完成（同时驱动本值的本地队列）；
//! - `delay`：睡眠 / 延迟执行，以及计时能力（[`TrClock`] / [`TrTime`]）；
//! - `spawn_send`：投递任务到全局工作队列；
//! - `local_scope`：值的本地队列（`!Send` 任务 + 统一驱动入口）；
//! - `spawn_blocking`：投递阻塞函数到阻塞线程池。
//!
//! 所有实现都基于基础 crate [`abs_art`] 中的 trait。
//!
//! # 运行时是**值**——但 smol 只有一半能被值钉住
//!
//! [`Runtime`] 是一个真正的运行时值：它在 `local_scope` feature 开启时持有一条
//! 自己的本地队列（`Rc<LocalExecutor<'static>>`）。因此
//!
//! - `spawn_local` / `run_until` / `block_on` 打在**这个值**持有的那条本地队列上；
//! - `delay` / `now` / `interval` 由**这个值**给出（`TrTime: TrDelay + TrClock`），
//!   因此「睡在哪个时间基准上、读的是哪个时刻」不可能被指向别处。
//!
//! **`spawn` 是唯一的例外**：smol 的全局执行器是**进程级单例**
//! （`smol::spawn` 内部用 `OnceCell<Executor>` 懒初始化，交给后台线程永久驱动，
//! 见 smol 2.0.2 的 `src/spawn.rs`），它既不能被替换、也不能被 drop，因此**没有
//! 任何值能持有它**。本 crate 的选择是：`TrSpawnSend::spawn` 如实转发到那个全局
//! 执行器，并把这条限制写进文档，而不是伪造一个「每值一份」的假象。
//!
//! 这条限制的可观测后果（`abs_art-smol` 的测试里钉住了第一条）：
//!
//! 1. **值被 drop 不会停止已投递的全局任务**：任务归进程级执行器所有；
//! 2. **同一个进程里的两个运行时值的 `spawn` 共享同一条全局队列**：它们之间没有
//!    队列边界，`Runtime` 值的数量不改变全局并发度；
//! 3. **全局并发度由环境变量 `SMOL_THREADS` 决定**（缺省 1 个后台线程），它是进程
//!    级配置，不是本值的能力。
//!
//! 需要「队列随值走」的隔离语义时，用 [`TrLocalScope::spawn_local`]——那才是本值
//! 真正拥有的队列。这也是本 crate 与 `abs_art-tokio` 唯一的形状差异：tokio 的
//! `Handle` 是一个**可克隆的运行时把手**，值能钉住它；smol 没有对应物。
//!
//! 代价与 tokio 一致：值必须被**构造**出来，且当它持有本地队列时是 `!Send`
//! ——本地队列本来就绑定线程。
//!
//! # 两种用法
//!
//! ## 具体用法（二进制层）
//!
//! ```
//! use abs_art::TrBlockOn;
//!
//! let rt = abs_art_smol::current();
//! let out = rt.block_on(async { 42 });
//! assert_eq!(out, 42);
//! ```
//!
//! ## 能力位用法（业务库层，编译期能力检查）
//!
//! 通过 const 泛型声明所需能力；请求了未声明（或本后端不支持）的能力会在
//! 编译期报错：
//!
//! ```
//! use abs_art::{BLOCK_ON, SPAWN_SEND, TrBlockOn, TrSpawnSend};
//! use abs_art_smol::Runtime;
//!
//! // 只声明 block_on + spawn_send 两种能力
//! let rt = Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current();
//! let handle = rt.spawn(async { 2 });
//! assert_eq!(rt.block_on(handle).unwrap(), 2);
//! ```
//!
//! ```compile_fail
//! use abs_art::{BLOCK_ON, TrSpawnSend};
//! use abs_art_smol::Runtime;
//!
//! // 只声明了 block_on 能力，spawn（spawn_send）不可用 → 编译错误（Tag 严格模式）
//! let rt = Runtime::<{ BLOCK_ON }>::current();
//! let _ = rt.spawn(async { 1 });
//! ```
//!
//! ## 本地投递（`!Send` 任务）
//!
//! 本地队列归运行时**值**所有，投递与驱动都在同一个值上：
//!
//! ```
//! use abs_art::{TrBlockOn, TrLocalScope};
//!
//! let rt = abs_art_smol::current();
//! let out = rt.block_on(rt.run_until(async {
//!     let rc = std::rc::Rc::new(6u32); // !Send：只有本地队列能承载
//!     rt.spawn_local(async move { *rc * 7 }).await.unwrap()
//! }));
//! assert_eq!(out, 42);
//! ```

#![no_std]

extern crate alloc;

// smol 2.x 无条件依赖 `std`（`smol::spawn` 起后台线程、`blocking` 用线程池），
// 本 crate 沿用 `no_std` 写法只是「不注入 std prelude」。但 `TrClock::Instant`
// 需要写出 `std::time::Instant`——那正是 async-io 计时器所用的时间基准
// （`async_io::Timer::after` 的到期时刻就是按 `std::time::Instant` 算的），
// 因此必须显式引入 `std`。它**不新增**任何依赖。
extern crate std;

// 只在 `local_scope` 下需要：那是本 crate 唯一持有堆上值的字段。
#[cfg(feature = "local_scope")]
use alloc::rc::Rc;

// `RuntimeTag` 与基础 crate 的其它公开项一起重导出（保持本 crate 原有的公开面）。
pub use abs_art::{
    BLOCK_ON, DELAY, Elapsed, FULL, RuntimeTag, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND,
    Timeout, TrAsyncRuntime, TrBlockOn, TrClock, TrDelay, TrInterval, TrJoinHandle,
    TrLocalScope, TrSpawnBlocking, TrSpawnSend, TrTime, UnitFuture,
};

/// smol 组合运行时**值**。
///
/// 对应基础 crate 中的 [`RuntimeTag::Smol`]。它在 `local_scope` feature 开启时持有
/// 一条自己的本地队列（`Rc<LocalExecutor<'static>>`）。
///
/// 类型参数 `CAPS` 是能力位掩码（见 [`abs_art::caps`]）：默认 [`FULL`]（全功能），
/// 也可以写成 `Runtime<{ BLOCK_ON | SPAWN_SEND }>` 只声明部分能力。掩码决定这个
/// **类型**实现了哪些能力 trait，从而决定哪些方法可调。
///
/// # 构造
///
/// 见 [`Runtime::current`]：smol 没有「环境运行时」可捕获，因此构造**没有先决
/// 条件**，任何线程都能直接构造（这一点与 `abs_art-tokio` 的 `Handle::current()`
/// 不同）。需要默认能力位时用 crate 级自由函数 [`current`]。
///
/// # 值钉住了什么、没钉住什么
///
/// | 能力 | 载体 | 值能否钉住 |
/// | --- | --- | --- |
/// | `spawn_local` / `run_until` | 本值持有的 `Rc<LocalExecutor>` | **能** |
/// | `delay` / `now` / `interval` | async-io 的**进程级反应器**；值给出 `Instant` 与调用形状 | **部分**：反应器是进程级资源，但 `TrTime: TrDelay + TrClock` 让「睡在哪个基准、读哪个时刻」由同一个值回答 |
/// | `spawn` | `smol::spawn` 的进程级全局执行器（`OnceCell`） | **不能**（见 crate 文档） |
/// | `spawn_blocking` | `blocking` 的进程级线程池 | **不能**（线程池是进程级资源） |
///
/// # 克隆
///
/// `Clone` 共享**同一条**本地队列——克隆出来的值与原来的值是同一个运行时的两个
/// 把手，不是两份运行时。
pub struct Runtime<const CAPS: usize = FULL> {
    /// 本值自己的本地队列（本地投递与 `run_until` / `block_on` 的驱动对象）。
    #[cfg(feature = "local_scope")]
    local_: Rc<smol::LocalExecutor<'static>>,
}

/// 构造**全能力**（`Runtime<FULL>`）运行时值。
///
/// 这是最常用的构造入口：类型参数 `CAPS` 直接取默认值 [`FULL`]，因此在表达式
/// 位置也**不需要类型标注**（`Runtime::current()` 在表达式位置会因 `CAPS` 无法
/// 推断而报 `E0284`，自由函数不会）。需要显式声明能力时用
/// [`Runtime::current`] 的 turbofish 形式。
///
/// # Panics
///
/// 不 panic：smol 没有「环境运行时」这一前提。
///
/// # Examples
///
/// ```
/// let rt = abs_art_smol::current();
/// assert_eq!(rt.tag(), abs_art::RuntimeTag::Smol);
/// ```
pub fn current() -> Runtime {
    Runtime::current()
}

impl<const CAPS: usize> Runtime<CAPS> {
    /// 构造运行时值。
    ///
    /// smol 没有「环境运行时句柄」可捕获：全局执行器是进程级单例（见 crate 文档），
    /// 本地队列则**本来就归每个运行时值所有**。因此本函数没有先决条件，也不会捕获
    /// 任何环境状态——每次调用都得到一个**新值**（`local_scope` 开启时含一条新的
    /// 空本地队列），全局执行器不受影响。
    ///
    /// # Panics
    ///
    /// 不 panic（与 `abs_art-tokio` 的 `Runtime::current` 不同，后者要求调用点处于
    /// tokio 运行时上下文内）。
    ///
    /// # Examples
    ///
    /// 注意表达式位置必须写全类型（默认的 `CAPS = FULL` **不参与**表达式位置的
    /// 推断，`Runtime::current()` 会因 const 泛型无法推断而报 `E0284`）；需要默认
    /// 能力位时用 crate 级自由函数 [`current`]。
    ///
    /// ```
    /// use abs_art_smol::Runtime;
    ///
    /// let value = Runtime::<{ abs_art::FULL }>::current();
    /// assert_eq!(value.tag(), abs_art::RuntimeTag::Smol);
    /// ```
    pub fn current() -> Self {
        Self {
            #[cfg(feature = "local_scope")]
            local_: Rc::new(smol::LocalExecutor::new()),
        }
    }

    /// 复制一份把手：与本值共享同一条本地队列。
    ///
    /// 与 `Clone` 等价，但可以在 CAPS 上「换标签」。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art_smol::Runtime;
    ///
    /// let a = Runtime::<{ abs_art::FULL }>::current();
    /// let b = a.retag::<{ abs_art::FULL }>();
    /// assert_eq!(b.tag(), abs_art::RuntimeTag::Smol);
    /// ```
    pub fn retag<const OTHER: usize>(&self) -> Runtime<OTHER> {
        Runtime {
            #[cfg(feature = "local_scope")]
            local_: Rc::clone(&self.local_),
        }
    }

    /// 本值对应的抽象运行时标签。
    pub fn tag(&self) -> RuntimeTag {
        RuntimeTag::Smol
    }
}

impl<const CAPS: usize> Clone for Runtime<CAPS> {
    fn clone(&self) -> Self {
        self.retag()
    }
}

impl<const CAPS: usize> core::fmt::Debug for Runtime<CAPS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("abs_art_smol::Runtime")
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
mod local_scope;

#[cfg(feature = "spawn_blocking")]
mod spawn_blocking;
