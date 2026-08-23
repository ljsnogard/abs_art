//! `abs_art` 基础 crate：异步运行时的抽象层。
//!
//! 本 crate **不依赖任何异步运行时**，只提供：
//!
//! - [`runtime::Runtime`]：抽象的运行时标签；
//! - [`runtime`] 中的一组能力 trait（`TrBlockOn` / `TrSpawnSend` / `TrSpawnLocal`
//!   / `TrSpawnBlocking` / `TrDelay`）；
//! - [`caps`]：能力位掩码与类型级标记，供组合 crate 的 `Runtime<const CAPS>`
//!   做编译期能力检查。
//!
//! 具体的运行时实现由组合 crate 提供：
//!
//! - [`abs_art_tokio`](https://docs.rs/abs_art-tokio)：tokio 后端；
//! - [`abs_art_compio`](https://docs.rs/abs_art-compio)：compio 后端；
//! - [`abs_art_smol`](https://docs.rs/abs_art-smol)：smol 后端。
//!
//! 每个组合 crate 都把 `block_on` / `delay` / `spawn_send` / `spawn_local` /
//! `spawn_blocking` 五个功能做成 feature 开关，用户按需启用。

#![no_std]

pub mod caps;
pub mod runtime;

pub use caps::{
    BLOCK_ON, DELAY, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, HasBlockOn,
    HasDelay, HasSpawnBlocking, HasSpawnLocal, HasSpawnSend,
};
pub use runtime::{
    Runtime, TrAsyncRuntime, TrBlockOn, TrDelay, TrJoinHandle, TrSpawnBlocking,
    TrSpawnLocal, TrSpawnSend,
};
