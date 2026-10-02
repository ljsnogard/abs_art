//! `abs_art` 基础 crate：异步运行时的抽象层。
//!
//! 本 crate **不依赖任何异步运行时**，只提供：
//!
//! - [`runtime::Runtime`]：抽象的运行时标签；
//! - [`runtime`] 中的一组能力 trait（`TrBlockOn` / `TrSpawnSend`
//!   / `TrSpawnBlocking` / `TrDelay`）与值化的本地作用域 trait
//!   [`TrLocalScope`]；
//! - [`caps`]：能力位掩码与类型级标记，供组合 crate 的 `Runtime<const CAPS>`
//!   做编译期能力检查。
//!
//! 注意**本地投递是「位 + 值」两件套**：能力位 [`SPAWN_LOCAL`] 只负责
//! **声明**（想用就必须写下来，使这次升级可被审查），真正能不能投递则由值
//! [`TrLocalScope`] 决定（它携带环境前提：本地队列存在且有人驱动）。两者的职责
//! 分工见 [`caps`] 模块文档。
//!
//! 具体的运行时实现由组合 crate 提供：
//!
//! - [`abs_art_tokio`](https://docs.rs/abs_art-tokio)：tokio 后端；
//! - [`abs_art_compio`](https://docs.rs/abs_art-compio)：compio 后端；
//! - [`abs_art_smol`](https://docs.rs/abs_art-smol)：smol 后端。
//!
//! 每个组合 crate 都把 `block_on` / `delay` / `spawn_send` / `local_scope` /
//! `spawn_blocking` 五个功能做成 feature 开关，用户按需启用。

#![no_std]

pub mod caps;
pub mod runtime;

pub use caps::{
    BLOCK_ON, DELAY, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, HasBlockOn,
    HasDelay, HasSpawnBlocking, HasSpawnLocal, HasSpawnSend,
};
pub use runtime::{
    RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay, TrJoinHandle, TrLocalScope,
    TrSpawnBlocking, TrSpawnSend,
};
