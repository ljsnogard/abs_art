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
//! `TokioRuntime` / `CompioRuntime` / `SmolRuntime` 取用（按 feature 出现）。
//! 一个都没启用时 `compile_error!`（fail fast）。**缺省是 compio**
//! （`default = ["default-backend-compio"]`，即「显式声明默认」，这样多后端并集时
//! 裸名仍有唯一解）。
//!
//! # 业务代码用法
//!
//! 本版起运行时是**值**：先构造它，再在它上面调能力方法。
//!
//! ```no_run
//! use core::time::Duration;
//!
//! use abs_art_bridge::{BLOCK_ON, DELAY, Runtime, TrBlockOn, TrDelay};
//!
//! # fn drive<T>(_: impl FnOnce() -> T) -> T { unimplemented!() }
//! # let out: u32 = drive(|| {
//! // 构造运行时**值**（需要已处于后端运行时上下文内）
//! let rt = Runtime::<{ BLOCK_ON | DELAY }>::current();
//! // 声明的能力决定哪些方法可调；未声明（或后端不支持）的能力在编译期报错
//! rt.block_on(async {
//!     rt.delay(Duration::from_millis(1)).await;
//!     2u32
//! })
//! # });
//! ```
//!
//! 上面刻意只用**跨后端共同**的能力：全局 `spawn`（[`TrSpawnSend`]）只有具备
//! 跨线程工作队列的后端才提供（tokio / smol），**compio 不实现它**——compio 上
//! 投递任务请走下面那条本地作用域路径。跨后端共用的代码因此只应约束共同子集：
//! [`TrBlockOn`] + [`TrDelay`] / [`TrTime`] / [`TrClock`] + [`TrSpawnBlocking`] +
//! [`TrLocalScope`]。
//!
//! 本地投递（`!Send` 任务）需要一个**本线程队列的别名**，由运行时值交出。需要
//! 「同步等到 future 完成、同时不饿死本地队列」时，用作用域自己的
//! [`TrLocalScope::block_on_local`]：
//!
//! ```no_run
//! use abs_art_bridge::{FULL, Runtime, TrLocalScope};
//!
//! let rt = Runtime::<{ FULL }>::current();
//! let scope = rt.local_scope();           // 要求 CAPS 含 SPAWN_LOCAL
//! let out = scope.block_on_local(async {
//!     let rc = std::rc::Rc::new(1u32);    // !Send：只有本地队列能承载
//!     scope.spawn_local(async move { *rc }).await.unwrap()
//! });
//! # let _ = out;
//! ```
//!
//! 它**不使用**运行时的阻塞原语（tokio 的 `block_in_place` 在 `LocalSet` 内被禁止），
//! 而是自己驱动本线程队列；与 [`TrBlockOn::block_on`] 的分工、以及各后端的边界见该
//! 方法的文档。外层已经有正在跑的驱动源时，也可以照旧写
//! `rt.block_on(scope.run_until(f))` 或直接 `scope.run_until(f).await`。

#![no_std]

#[cfg(test)]
extern crate std;

pub use abs_art::{
    BLOCK_ON, CLOCK, DELAY, RuntimeTag, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, TrAsyncRuntime,
    TrBlockOn, TrClock, TrDelay, TrJoinHandle, TrLocalScope, TrMockClock, TrSpawnBlocking,
    TrSpawnSend, TrTime,
};

// ── 「完整能力集」按**后端**给，而不是按位集合给 ───────────────────────────
//
// `abs_art::FULL` 是「所有位」；但每个后端**实现得了的**位不同：compio 没有跨线程
// 全局队列，它的 `FULL` 不含 `SPAWN_SEND`。所以这里：
// - 裸名 `FULL` = **当前默认后端**的完整能力集；
// - 具名 `TokioFull` / `CompioFull` / `SmolFull` = 各后端自己的完整能力集
//   （只要该后端的 feature 开启就存在，因此 `cargo test --workspace` 的 feature
//   并集下仍然精确）。
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
pub use abs_art_tokio::FULL;

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
pub use abs_art_compio::FULL;

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
pub use abs_art_smol::FULL;

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

/// 当前**默认**后端交出的本地作用域类型（线程独占，`!Send`）。
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
pub use abs_art_tokio::LocalScope;

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

/// 当前**默认**后端交出的本地作用域类型（线程独占，`!Send`）。
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
pub use abs_art_compio::LocalScope;

/// 当前**默认**后端提供的运行时类型（仅在未启用前两者时来自 smol）。
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

/// 当前**默认**后端交出的本地作用域类型（线程独占，`!Send`）。
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
pub use abs_art_smol::LocalScope;

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

/// 具名别名：tokio 后端的完整能力集。
#[cfg(feature = "backend-tokio")]
pub use abs_art_tokio::FULL as TokioFull;

/// 具名别名：compio 后端的完整能力集（不含 `SPAWN_SEND`）。
#[cfg(feature = "backend-compio")]
pub use abs_art_compio::FULL as CompioFull;

/// 具名别名：smol 后端的完整能力集。
#[cfg(feature = "backend-smol")]
pub use abs_art_smol::FULL as SmolFull;

/// 具名别名：tokio 后端的本地作用域类型。
#[cfg(feature = "backend-tokio")]
pub use abs_art_tokio::LocalScope as TokioLocalScope;

/// 具名别名：compio 后端的本地作用域类型。
#[cfg(feature = "backend-compio")]
pub use abs_art_compio::LocalScope as CompioLocalScope;

/// 具名别名：smol 后端的本地作用域类型。
#[cfg(feature = "backend-smol")]
pub use abs_art_smol::LocalScope as SmolLocalScope;

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
compile_error!(
    "abs_art-bridge：必须启用一个 backend feature（backend-tokio / backend-compio / backend-smol）"
);

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
    //! tokio 后端下的桥接烟雾测试。
    //!
    //! 刻意用**具名别名** `TokioRuntime` 而不是裸名 `Runtime`：裸名跟着 bridge 的
    //! 缺省后端走，而 `cargo test --workspace` 会把 bridge 的 feature 取并集
    //! （缺省后端 + 下游所用的后端）。用别名之后，本模块无论缺省是谁都在测 tokio。

    use super::{
        BLOCK_ON, RuntimeTag, SPAWN_LOCAL, TokioRuntime as Runtime, TrBlockOn, TrLocalScope,
    };

    /// 目的：验证桥接 crate 在启用 `backend-tokio` 时，`TokioRuntime` 与
    /// `TokioLocalScope` 确实解析为 tokio 后端的类型，且能力位与本地投递可用。
    ///
    /// 手段：在运行时上下文**之外**用 `with_handle` 构造声明了
    /// `BLOCK_ON | SPAWN_LOCAL` 的值，由它交出本地作用域，再用
    /// `rt.block_on(scope.run_until(..))` 驱动一个 `!Send` 任务；同时比较 `tag()` 与
    /// 抽象标签。
    ///
    /// 判断：`tag()` 等于 [`RuntimeTag::Tokio`]，且本地任务取回 42。
    #[test]
    fn tokio_backend_resolves() {
        let outer = tokio::runtime::Runtime::new().unwrap();
        let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::with_handle(outer.handle().clone());
        assert_eq!(rt.tag(), RuntimeTag::Tokio);

        let scope = rt.local_scope();
        let out = rt.block_on(scope.run_until(async {
            let rc = std::rc::Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证 tokio 的运行时值不携带本地队列——它是 `Send + Sync` 的把手。
    ///
    /// 手段：编译期断言 `Runtime<{ FULL }>: Send + Sync`。
    ///
    /// 判断：编译通过即为通过（本地队列在 `LocalScope` 上，见本 crate 文档）。
    #[test]
    fn tokio_runtime_value_is_send_sync() {
        use abs_art::FULL;
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime<{ FULL }>>();
    }
}

#[cfg(all(test, feature = "backend-compio"))]
mod tests_compio_ {
    //! compio 后端下的桥接烟雾测试。
    //!
    //! 与 tokio 模块同样用**具名别名** `CompioRuntime`，因此不受 bridge 缺省后端影响。

    use super::{BLOCK_ON, CompioRuntime as Runtime, RuntimeTag, SPAWN_LOCAL, TrLocalScope};

    /// 目的：验证桥接 crate 在启用 `backend-compio` 时，`CompioRuntime` 与
    /// `CompioLocalScope` 解析正确，且本地投递可用。
    ///
    /// 手段：在 compio 运行时上下文内构造声明了 `BLOCK_ON | SPAWN_LOCAL` 的值，
    /// 由它交出本地作用域，用 `run_until` 驱动一个捕获 `Rc` 的 `!Send` 任务
    /// （compio 的 `run_until` 等价于直接 await：队列由运行时自己驱动）。
    ///
    /// 判断：`tag()` 等于 [`RuntimeTag::Compio`]，且本地任务取回 42。
    #[test]
    fn compio_backend_resolves() {
        let outer = compio::runtime::Runtime::new().unwrap();

        let out = outer.block_on(async {
            let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
            assert_eq!(rt.tag(), RuntimeTag::Compio);

            let scope = rt.local_scope();
            scope
                .run_until(async {
                    let rc = std::rc::Rc::new(6u32);
                    scope.spawn_local(async move { *rc * 7 }).await.unwrap()
                })
                .await
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证 compio 的运行时值**不实现** `TrSpawnSend` ——桥接层面也拿不到
    /// 全局 `spawn`。
    ///
    /// 手段：本测试**不**调用 `rt.spawn(..)`（那会编译失败，见
    /// `abs_art-compio` 的 `compile_fail` 文档测试）；改为断言「投递只能经本地
    /// 作用域」，即上面的 `compio_backend_resolves`，并在本注释里记录这条边界。
    ///
    /// 判断：`<Runtime<{ FULL }> as TrSpawnSend>` 的负向事实无法用正断言表达，
    /// 这里用**编译期**方式钉住它的正面：运行时值实现了 `TrLocalScope` 的入口
    /// （`local_scope()`），而 `TrSpawnSend` 由 `abs_art-compio` 侧的反例负责。
    #[test]
    fn compio_local_scope_is_the_only_delivery_path() {
        use abs_art::FULL;
        fn assert_local_scope_entry<const CAPS: usize>()
        where
            [(); CAPS]: abs_art::HasSpawnLocal,
        {
        }
        assert_local_scope_entry::<{ FULL }>();
    }
}

#[cfg(all(test, feature = "backend-smol"))]
mod tests_smol_ {
    //! smol 后端下的桥接烟雾测试。
    //!
    //! 同样用**具名别名** `SmolRuntime`。

    use super::{
        BLOCK_ON, RuntimeTag, SPAWN_LOCAL, SmolRuntime as Runtime, TrBlockOn, TrLocalScope,
    };

    /// 目的：验证桥接 crate 在启用 `backend-smol` 时，`SmolRuntime` 与
    /// `SmolLocalScope` 解析正确，本地投递可用，且运行时值是 `Send + Sync` 的
    /// 零大小标记（不携带队列）。
    ///
    /// 手段：直接构造值（smol 无环境运行时前提），用 `rt.block_on(scope.run_until(..))`
    /// 驱动一个 `!Send` 任务；另加编译期 `Send + Sync` 断言。
    ///
    /// 判断：`tag()` 等于 [`RuntimeTag::Smol`]，本地任务取回 42，断言编译通过。
    #[test]
    fn smol_backend_resolves() {
        use abs_art::FULL;

        let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
        assert_eq!(rt.tag(), RuntimeTag::Smol);

        let scope = rt.local_scope();
        let out = rt.block_on(scope.run_until(async {
            let rc = std::rc::Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));
        assert_eq!(out, 42);

        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime<{ FULL }>>();
    }
}
