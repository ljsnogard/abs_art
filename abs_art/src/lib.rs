//! `abs_art` 基础 crate：异步运行时的抽象层。
//!
//! 本 crate **不依赖任何异步运行时**，只提供：
//!
//! - [`runtime::RuntimeTag`]：抽象的运行时身份标签；
//! - [`runtime`] 中的一组**值化**能力 trait（`TrAsyncRuntime` / `TrBlockOn` /
//!   `TrSpawnSend` / `TrSpawnBlocking` / `TrDelay` / `TrLocalScope`）：它们的
//!   方法都收 `&self`，由**运行时值**提供能力；
//! - [`time`]：计时能力 trait（[`TrClock`] / [`TrTime`] / [`TrInterval`]）与超时
//!   失败类型 [`Elapsed`]、以及组合出超时的具体类型 [`Timeout`]；
//! - [`caps`]：能力位掩码与类型级标记，供组合 crate 的 `Runtime<const CAPS>`
//!   做编译期能力检查。
//!
//! 注意**本地投递是「位 + 值」两件套**：能力位 [`SPAWN_LOCAL`] 只负责
//! **声明**（想用就必须写下来，使这次升级可被审查），真正能不能投递则由
//! **运行时值**决定（它携带环境前提：本地队列存在且有人驱动）。两者的职责
//! 分工见 [`caps`] 与 [`TrLocalScope`] 的文档。
//!
//! 具体的运行时实现由组合 crate 提供：
//!
//! - [`abs_art_tokio`](https://docs.rs/abs_art-tokio)：tokio 后端；
//! - [`abs_art_compio`](https://docs.rs/abs_art-compio)：compio 后端；
//! - [`abs_art_smol`](https://docs.rs/abs_art-smol)：smol 后端。
//!
//! 每个组合 crate 都把 `block_on` / `delay` / `spawn_send` / `local_scope` /
//! `spawn_blocking` 五个功能做成 feature 开关，用户按需启用。
//!
//! # 本版：能力收 `&self`，运行时**值化**
//!
//! 运行时不再只是类型标签：它是有能力的**值**。业务库把 `&R`
//! （`R: TrSpawnSend + TrTime + …`）拿在手上，`spawn` / `block_on` / `delay` /
//! `now` 都作用在这一个值上，因此「哪个运行时、哪个时钟」不再可能被指向别处。
//!
//! **本地队列不在运行时值里**：它是**线程独占**的资源，由 [`TrLocalScope`] 承载
//! （各后端的 `LocalScope`，经 `Runtime<CAPS>::local_scope()` 取得，且要求 `CAPS`
//! 含 [`SPAWN_LOCAL`]）。计时与时刻属于运行时值；`spawn_local` / `run_until` 属于
//! 作用域。为什么这样分、以及曾经合在一起的代价，见 [`runtime`] 模块文档。

#![no_std]

#[cfg(test)]
extern crate std;

pub mod caps;
pub mod runtime;
pub mod time;

pub use caps::{
    BLOCK_ON, CLOCK, DELAY, FULL, HasBlockOn, HasClock, HasDelay, HasSpawnBlocking, HasSpawnLocal,
    HasSpawnSend, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND,
};
pub use runtime::{
    RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay, TrJoinHandle, TrLocalScope, TrSpawnBlocking,
    TrSpawnSend,
};
pub use time::{Elapsed, Timeout, TrClock, TrInterval, TrMockClock, TrTime, UnitFuture};
