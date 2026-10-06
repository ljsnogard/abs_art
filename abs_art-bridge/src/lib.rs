//! `abs_art` 的桥接（bridge）crate。
//!
//! 让**业务代码**只依赖这一个 crate 就能使用运行时**值**，而不直接依赖任何后端
//! crate（`abs_art-tokio` / `abs_art-compio` / `abs_art-smol`）。
//!
//! # 后端选择（通过 Cargo.toml，而非代码）
//!
//! 集成方（通常是最终的二进制）在 Cargo.toml 里启用 `backend-*` feature：
//!
//! ```toml
//! [dependencies]
//! abs_art-bridge = { path = "abs_art-bridge", features = ["backend-tokio"] }
//! ```
//!
//! 切换后端 = 改这一行 feature，业务代码零改动。
//!
//! # 多个后端可以同时启用
//!
//! 裸名 [`Runtime`] 按 **cfg 优先级**解析（`backend-tokio` > `backend-compio` >
//! `backend-smol`）；同时启用多个后端时，非默认的那些用**具名别名**
//! [`TokioRuntime`] / [`CompioRuntime`] / [`SmolRuntime`] 取用。
//! 一个都没启用时 `compile_error!`（fail fast）。
//!
//! # 业务代码用法
//!
//! v0.4 起运行时是**值**：先构造它，再在它上面调能力方法。
//!
//! ```no_run
//! use abs_art_bridge::{BLOCK_ON, Runtime, SPAWN_SEND, TrBlockOn, TrSpawnSend};
//!
//! # fn drive<T>(_: impl FnOnce() -> T) -> T { unimplemented!() }
//! # let out: u32 = drive(|| {
//! let rt = Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current();
//! // 声明的能力决定哪些方法可调；未声明（或后端不支持）的能力在编译期报错
//! let _handle = rt.spawn(async { 1u32 });
//! rt.block_on(async { 2u32 })
//! # });
//! ```
//!
//! 本地投递（`!Send` 任务）不再是独立的作用域值，而是**运行时值自己**的能力：
//!
//! ```no_run
//! use abs_art_bridge::{FULL, Runtime, TrBlockOn, TrLocalScope};
//!
//! let rt = Runtime::<{ FULL }>::current();
//! let out = rt.block_on(rt.run_until(async {
//!     let rc = std::rc::Rc::new(1u32); // !Send：只有本地队列能承载
//!     rt.spawn_local(async move { *rc }).await.unwrap()
//! }));
//! # let _ = out;
//! ```

#![no_std]

#[cfg(test)]
extern crate std;

pub use abs_art::{
    BLOCK_ON, DELAY, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, RuntimeTag,
    TrAsyncRuntime, TrBlockOn, TrClock, TrDelay, TrJoinHandle, TrLocalScope,
    TrSpawnBlocking, TrSpawnSend, TrTime,
};

/// 当前**默认**后端提供的运行时类型。
///
/// 选择规则：
/// - 只启用**一个** backend 时，它就是默认（不必声明）；
/// - 启用**多个** backend 时，必须显式启用 `default-backend-*` 之一，否则编译失败。
#[cfg(any(
    all(
        feature = "default-backend-tokio",
        not(feature = "default-backend-compio"),
        not(feature = "default-backend-smol"),
    ),
    all(
        not(any(
            feature = "default-backend-tokio",
            feature = "default-backend-compio",
            feature = "default-backend-smol",
        )),
        feature = "backend-tokio",
        not(feature = "backend-compio"),
        not(feature = "backend-smol"),
    ),
))]
pub use abs_art_tokio::Runtime;

/// 当前**默认**后端提供的运行时类型（compio 版）。
#[cfg(any(
    all(
        feature = "default-backend-compio",
        not(feature = "default-backend-tokio"),
        not(feature = "default-backend-smol"),
    ),
    all(
        not(any(
            feature = "default-backend-tokio",
            feature = "default-backend-compio",
            feature = "default-backend-smol",
        )),
        not(feature = "backend-tokio"),
        feature = "backend-compio",
        not(feature = "backend-smol"),
    ),
))]
pub use abs_art_compio::Runtime;

/// 当前**默认**后端提供的运行时类型（smol 版）。
#[cfg(any(
    all(
        feature = "default-backend-smol",
        not(feature = "default-backend-tokio"),
        not(feature = "default-backend-compio"),
    ),
    all(
        not(any(
            feature = "default-backend-tokio",
            feature = "default-backend-compio",
            feature = "default-backend-smol",
        )),
        not(feature = "backend-tokio"),
        not(feature = "backend-compio"),
        feature = "backend-smol",
    ),
))]
pub use abs_art_smol::Runtime;

/// 当前**默认**后端的「用运行时上下文构造全能力值」入口。
///
/// 等价于 `Runtime::<{ FULL }>::current()`，但**不需要写类型参数**，因此在表达式
/// 位置也不会遇到类型推断问题。
#[cfg(any(
    all(
        feature = "default-backend-tokio",
        not(feature = "default-backend-compio"),
        not(feature = "default-backend-smol"),
    ),
    all(
        not(any(
            feature = "default-backend-tokio",
            feature = "default-backend-compio",
            feature = "default-backend-smol",
        )),
        feature = "backend-tokio",
        not(feature = "backend-compio"),
        not(feature = "backend-smol"),
    ),
))]
pub use abs_art_tokio::current;

/// 当前**默认**后端的构造入口（compio 版）。
#[cfg(any(
    all(
        feature = "default-backend-compio",
        not(feature = "default-backend-tokio"),
        not(feature = "default-backend-smol"),
    ),
    all(
        not(any(
            feature = "default-backend-tokio",
            feature = "default-backend-compio",
            feature = "default-backend-smol",
        )),
        not(feature = "backend-tokio"),
        feature = "backend-compio",
        not(feature = "backend-smol"),
    ),
))]
pub use abs_art_compio::current;

/// 当前**默认**后端的构造入口（smol 版）。
#[cfg(any(
    all(
        feature = "default-backend-smol",
        not(feature = "default-backend-tokio"),
        not(feature = "default-backend-compio"),
    ),
    all(
        not(any(
            feature = "default-backend-tokio",
            feature = "default-backend-compio",
            feature = "default-backend-smol",
        )),
        not(feature = "backend-tokio"),
        not(feature = "backend-compio"),
        feature = "backend-smol",
    ),
))]
pub use abs_art_smol::current;

/// 具名别名：tokio 后端的运行时类型（只要 feature 开启就存在）。
#[cfg(feature = "backend-tokio")]
pub use abs_art_tokio::Runtime as TokioRuntime;

/// 具名别名：compio 后端的运行时类型。
#[cfg(feature = "backend-compio")]
pub use abs_art_compio::Runtime as CompioRuntime;

/// 具名别名：smol 后端的运行时类型。
#[cfg(feature = "backend-smol")]
pub use abs_art_smol::Runtime as SmolRuntime;

/// 具名别名：tokio 后端的 `JoinHandle`。
#[cfg(feature = "backend-tokio")]
pub use abs_art_tokio::JoinHandle as TokioJoinHandle;

/// 具名别名：compio 后端的 `JoinHandle`。
#[cfg(feature = "backend-compio")]
pub use abs_art_compio::JoinHandle as CompioJoinHandle;

/// 具名别名：smol 后端的 `JoinHandle`。
#[cfg(feature = "backend-smol")]
pub use abs_art_smol::JoinHandle as SmolJoinHandle;

#[cfg(not(any(
    feature = "backend-tokio",
    feature = "backend-compio",
    feature = "backend-smol",
)))]
compile_error!("abs_art-bridge：必须启用一个 backend feature（backend-tokio / backend-compio / backend-smol）");

// 守卫一：启用了多个 backend 却没声明默认 → 裸名会按优先级**悄悄**选一个，很可能是
// 错的那个（`cargo test --workspace` 的 feature 并集尤其容易踩到）。这里让它编译失败。
#[cfg(all(
    not(any(
        feature = "default-backend-tokio",
        feature = "default-backend-compio",
        feature = "default-backend-smol",
    )),
    any(
        all(feature = "backend-tokio", feature = "backend-compio"),
        all(feature = "backend-tokio", feature = "backend-smol"),
        all(feature = "backend-compio", feature = "backend-smol"),
    ),
))]
compile_error!(
    "abs_art-bridge：启用了多个 backend，必须显式声明默认后端\
     （default-backend-tokio / default-backend-compio / default-backend-smol 之一）；\
     否则裸名会按优先级悄悄选中一个，可能与你想要的运行时不一致。"
);

// 守卫二：声明了多个默认后端 → 裸名没有唯一解，编译失败。
#[cfg(any(
    all(feature = "default-backend-tokio", feature = "default-backend-compio"),
    all(feature = "default-backend-tokio", feature = "default-backend-smol"),
    all(feature = "default-backend-compio", feature = "default-backend-smol"),
))]
compile_error!(
    "abs_art-bridge：只能声明一个 default-backend-*（裸名 Runtime / current 必须唯一）。"
);

#[cfg(all(test, feature = "backend-tokio"))]
mod tests_tokio_ {
    //! tokio 后端下的桥接烟雾测试（`cargo test --workspace` 时运行）。

    use super::*;

    /// 目的：验证桥接 crate 在启用 `backend-tokio` 时，`Runtime` 确实解析为 tokio
    /// 后端的**运行时值**，且能力位与本地投递机制可用。
    ///
    /// 实施策略：在 tokio 运行时上下文内构造声明了 `BLOCK_ON | SPAWN_LOCAL` 的值，
    /// 用 `block_on` 驱动一个本地任务，并比较 `tag()` 与抽象标签。
    ///
    /// 通过依据：`tag()` 等于 [`RuntimeTag::Tokio`]，且本地任务取回 42。
    #[test]
    fn tokio_backend_resolves() {
        let outer = tokio::runtime::Runtime::new().unwrap();

        let out = outer.block_on(async {
            let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
            assert_eq!(rt.tag(), RuntimeTag::Tokio);

            rt.block_on(async {
                let rc = std::rc::Rc::new(6u32);
                rt.spawn_local(async move { *rc * 7 }).await.unwrap()
            })
        });

        assert_eq!(out, 42);
    }
}
