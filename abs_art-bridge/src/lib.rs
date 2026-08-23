//! `abs_art` 的桥接（bridge）crate。
//!
//! 让**业务代码**只依赖这一个 crate 就能使用 [`Runtime`]，而不直接依赖任何
//! 后端 crate（`abs_art-tokio` / `abs_art-compio` / `abs_art-smol`）。
//!
//! # 后端选择（通过 Cargo.toml，而非代码）
//!
//! 集成方（通常是最终的二进制）在 Cargo.toml 里启用且**只启用一个**
//! `backend-*` feature：
//!
//! ```toml
//! [dependencies]
//! abs_art-bridge = { path = "abs_art-bridge", features = ["backend-tokio"] }
//! ```
//!
//! 切换后端 = 改这一行 feature，业务代码零改动。
//!
//! # 业务代码用法
//!
//! ```no_run
//! use abs_art_bridge::{BLOCK_ON, SPAWN_LOCAL, Runtime, TrBlockOn, TrSpawnLocal};
//!
//! // 声明所需能力；未声明（或后端不支持）的能力在编译期报错
//! let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
//! let _ = rt;
//! ```

#![no_std]

#[cfg(test)]
extern crate std;

pub use abs_art::{
    BLOCK_ON, DELAY, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, Runtime as RuntimeTag,
    TrAsyncRuntime, TrBlockOn, TrDelay, TrJoinHandle, TrSpawnBlocking, TrSpawnLocal,
    TrSpawnSend,
};

/// 当前后端提供的 [`Runtime`] 类型（由 `backend-*` feature 决定）。
#[cfg(feature = "backend-tokio")]
pub use abs_art_tokio::Runtime;

/// 当前后端提供的 [`Runtime`] 类型（由 `backend-*` feature 决定）。
#[cfg(feature = "backend-compio")]
pub use abs_art_compio::Runtime;

/// 当前后端提供的 [`Runtime`] 类型（由 `backend-*` feature 决定）。
#[cfg(feature = "backend-smol")]
pub use abs_art_smol::Runtime;

#[cfg(not(any(
    feature = "backend-tokio",
    feature = "backend-compio",
    feature = "backend-smol",
)))]
compile_error!("abs_art-bridge：必须启用一个 backend feature（backend-tokio / backend-compio / backend-smol）");

#[cfg(any(
    all(feature = "backend-tokio", feature = "backend-compio"),
    all(feature = "backend-tokio", feature = "backend-smol"),
    all(feature = "backend-compio", feature = "backend-smol"),
    all(
        feature = "backend-tokio",
        feature = "backend-compio",
        feature = "backend-smol",
    ),
))]
compile_error!("abs_art-bridge：backend feature 只能启用一个");

#[cfg(all(test, feature = "backend-tokio"))]
mod tests_tokio {
    //! tokio 后端下的桥接烟雾测试（`cargo test --workspace` 时运行）。

    use super::*;

    /// 目的：验证桥接 crate 在启用 `backend-tokio` 时，`Runtime` 确实解析为
    /// tokio 后端的运行时类型，且 Tag 能力机制可用。
    ///
    /// 实施策略：比较 `Runtime::tag()` 与抽象标签，并构造一个声明了
    /// `BLOCK_ON | SPAWN_LOCAL` 能力的 `Runtime` 类型。
    ///
    /// 通过依据：`tag()` 等于 [`RuntimeTag::Tokio`]；Tag 类型构造成功
    /// （编译通过）即为通过。
    #[test]
    fn tokio_backend_resolves() {
        assert_eq!(Runtime::tag(), RuntimeTag::Tokio);

        let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
        let _ = rt;
    }
}
