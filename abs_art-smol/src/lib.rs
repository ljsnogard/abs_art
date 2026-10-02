//! `abs_art` 的 smol 组合 crate。
//!
//! 提供五个功能（各自为 feature 开关）：
//!
//! - `block_on`：阻塞等待一个 future 完成；
//! - `delay`：睡眠 / 延迟执行；
//! - `spawn_send`：投递任务到全局工作队列；
//! - `local_scope`：值化的本地作用域（`!Send` 任务 + 统一驱动入口）；
//! - `spawn_blocking`：投递阻塞函数到阻塞线程池。
//!
//! 所有实现都基于基础 crate [`abs_art`] 中的 trait。
//!
//! # 两种用法
//!
//! ## 具体用法（二进制层）
//!
//! ```
//! use abs_art_smol::Runtime;
//!
//! Runtime::block_on(async { 42 });
//! ```
//!
//! ## Tag 用法（业务库层，编译期能力检查）
//!
//! ```
//! use abs_art::{BLOCK_ON, SPAWN_SEND, TrBlockOn, TrSpawnSend};
//! use abs_art_smol::Runtime;
//!
//! let rt = Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current();
//! let _ = rt;
//! ```
//!
//! ```compile_fail
//! use abs_art::{BLOCK_ON, TrSpawnSend};
//! use abs_art_smol::Runtime;
//!
//! // 只声明了 block_on 能力，spawn（spawn_send）不可用 → 编译错误（Tag 严格模式）
//! let _ = <Runtime<{ BLOCK_ON }> as TrSpawnSend>::spawn(async { 1 });
//! ```
//!
//! ## 本地作用域（值，不是能力位）
//!
//! 本地投递自 v0.3 起由**值**承载。smol 2.x 不提供本地队列，因此 [`LocalScope`]
//! 自己持有一条 `LocalExecutor` 并负责驱动它：
//!
//! ```
//! use abs_art::TrLocalScope;
//! use abs_art_smol::LocalScope;
//!
//! let scope = LocalScope::new();
//! let out = smol::block_on(scope.run_until(async {
//!     let rc = std::rc::Rc::new(6u32); // !Send：只有本地队列能承载
//!     scope.spawn_local(async move { *rc * 7 }).await.unwrap()
//! }));
//! assert_eq!(out, 42);
//! ```

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub use abs_art::{
    BLOCK_ON, DELAY, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, RuntimeTag,
    TrAsyncRuntime,
    TrBlockOn, TrDelay, TrJoinHandle, TrLocalScope, TrSpawnBlocking, TrSpawnSend,
};

/// smol 组合运行时标记类型。
///
/// 对应基础 crate 中的 [`RuntimeTag::Smol`]。由于孤儿规则（trait 与类型都
/// 来自 `abs_art` 时无法在外部 crate 中为它实现 trait），每个组合 crate 都
/// 定义自己的本地 `Runtime` 类型，并为它实现 `abs_art` 中的全部 trait。
///
/// 类型参数 `CAPS` 是能力位掩码（见 [`abs_art::caps`]）：默认 [`FULL`]（全功能）。
/// **本地投递不在能力位里**，它由 [`LocalScope`] 这个值承载。
pub struct Runtime<const CAPS: usize = FULL>;

impl Runtime<FULL> {
    /// 返回本 crate 对应的抽象运行时标签。
    pub const fn tag() -> RuntimeTag {
        RuntimeTag::Smol
    }
}

impl<const CAPS: usize> Runtime<CAPS> {
    /// 返回当前运行时（零大小标记值）。
    ///
    /// 当 `CAPS` 未显式指定时（`Runtime::current()`），需要类型标注或通过
    /// 类型别名使用，例如 `let rt: Runtime = Runtime::current();`。
    pub const fn current() -> Self {
        Self
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

#[cfg(feature = "spawn_send")]
mod spawn_send;

#[cfg(feature = "local_scope")]
pub mod local_scope;

#[cfg(feature = "local_scope")]
pub use local_scope::LocalScope;

#[cfg(feature = "spawn_blocking")]
mod spawn_blocking;
