//! `abs_art` 的 compio 组合 crate。
//!
//! 提供五个功能（各自为 feature 开关）：
//!
//! - `block_on`：阻塞等待一个 future 完成；
//! - `delay`：睡眠 / 延迟执行；
//! - `spawn_send`：投递任务到运行时队列；
//! - `spawn_local`：投递任务到线程本地队列；
//! - `spawn_blocking`：投递阻塞函数到阻塞线程池。
//!
//! 所有实现都基于基础 crate [`abs_art`] 中的 trait。
//!
//! # 两种用法
//!
//! ## 具体用法（二进制层）
//!
//! ```
//! use abs_art_compio::Runtime;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! rt.block_on(async { Runtime::block_on(async { 42 }) });
//! ```
//!
//! ## Tag 用法（业务库层，编译期能力检查）
//!
//! ```
//! use abs_art::{BLOCK_ON, SPAWN_LOCAL, TrBlockOn, TrSpawnLocal};
//! use abs_art_compio::Runtime;
//!
//! let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
//! let _ = rt;
//! ```
//!
//! ```compile_fail
//! use abs_art::{BLOCK_ON, TrSpawnSend};
//! use abs_art_compio::Runtime;
//!
//! // 只声明了 block_on 能力，spawn（spawn_send）不可用 → 编译错误（Tag 严格模式）
//! let _ = <Runtime<{ BLOCK_ON }> as TrSpawnSend<_>>::spawn(async { 1 });
//! ```

#![no_std]

#[cfg(test)]
extern crate std;

pub use abs_art::{
    BLOCK_ON, DELAY, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, TrAsyncRuntime,
    TrBlockOn, TrDelay, TrJoinHandle, TrSpawnBlocking, TrSpawnLocal, TrSpawnSend,
};
pub use abs_art::Runtime as RuntimeTag;

/// compio 组合运行时标记类型。
///
/// 对应基础 crate 中的 [`RuntimeTag::Compio`]。由于孤儿规则（trait 与类型都
/// 来自 `abs_art` 时无法在外部 crate 中为它实现 trait），每个组合 crate 都
/// 定义自己的本地 `Runtime` 类型，并为它实现 `abs_art` 中的全部 trait。
///
/// 类型参数 `CAPS` 是能力位掩码（见 [`abs_art::caps`]）：默认 [`FULL`]（全功能），
/// 也可以写成 `Runtime<{ BLOCK_ON | SPAWN_LOCAL }>` 只声明部分能力。
pub struct Runtime<const CAPS: usize = FULL>;

impl Runtime<FULL> {
    /// 返回本 crate 对应的抽象运行时标签。
    pub const fn tag() -> RuntimeTag {
        RuntimeTag::Compio
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

#[cfg(feature = "spawn_local")]
mod spawn_local;

#[cfg(feature = "spawn_blocking")]
mod spawn_blocking;
