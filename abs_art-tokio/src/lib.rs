//! `abs_art` 的 tokio 组合 crate。
//!
//! 提供五个功能（各自为 feature 开关）：
//!
//! - `block_on`：阻塞等待一个 future 完成；
//! - `delay`：睡眠 / 延迟执行；
//! - `spawn_send`：投递任务到全局工作队列；
//! - `spawn_local`：投递任务到线程本地队列；
//! - `spawn_blocking`：投递阻塞函数到阻塞线程池。
//!
//! 所有实现都基于基础 crate [`abs_art`] 中的 trait。

#![no_std]

#[cfg(test)]
extern crate std;

pub use abs_art::{
    Runtime as RuntimeTag, TrBlockOn, TrDelay, TrSpawnBlocking, TrSpawnLocal,
    TrSpawnSend,
};

/// tokio 组合运行时标记类型。
///
/// 对应基础 crate 中的 [`RuntimeTag::Tokio`]。由于孤儿规则（trait 与类型都
/// 来自 `abs_art` 时无法在外部 crate 中为它实现 trait），每个组合 crate 都
/// 定义自己的本地 `Runtime` 类型，并为它实现 `abs_art` 中的全部 trait。
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Runtime;

impl Runtime {
    /// 返回本 crate 对应的抽象运行时标签。
    pub const fn tag() -> RuntimeTag {
        RuntimeTag::Tokio
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
