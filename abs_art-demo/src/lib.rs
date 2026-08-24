//! `abs_art-demo`：演示「业务库零泛型穿透 + 集成方通过 Cargo.toml 选后端」。
//!
//! - [`lib.rs`](crate)（业务库）：只依赖 [`abs_art_bridge`]（当前后端实例，
//!   见下），通过能力标签 `Runtime::<{ BLOCK_ON | SPAWN_SEND }>` 声明所需
//!   能力，**没有任何泛型参数穿透**；
//! - `main.rs`（二进制）：负责创建运行时并调用业务库——它是唯一允许感知
//!   后端的地方（创建哪个运行时的代码必须与所选后端一致）。
//!
//! # 后端选择（本 crate 的 features）
//!
//! `Cargo.toml` 里 `demo-tokio` 与 `demo-compio` **互斥**，一次构建只能启用
//! 一个：
//!
//! ```text
//! cargo run -p abs_art-demo                                        # tokio 组（默认）
//! cargo run -p abs_art-demo --no-default-features --features demo-compio   # compio 组
//! ```
//!
//! 本 crate 把同一个 `abs_art-bridge` 以两个 backend 实例化（重命名依赖
//! `bridge_tokio` / `bridge_compio`），`lib.rs` 用 `pub use ... as
//! abs_art_bridge` 把「当前后端实例」统一暴露为 `abs_art_bridge`——业务代码
//! 的写法与后端无关。
//!
//! # Examples（每种 cap 一个 smoke test，按后端分组）
//!
//! `examples/tokio_demo/` 与 `examples/compio_demo/` 各 7 个 demo，一一对应，
//! 用**同一种 cap 组合**验证 `abs_art` / `abs_art-bridge` 的一个设计意图，
//! 文档注释里写明「要验证什么 / 可以做到什么 / 不能做到什么」：
//!
//! - `cap_block_on.rs`：`BLOCK_ON` —— 最小能力 + `TrBlockOn` 放松 `'static`
//!   后可以驱动借用栈数据的 future / 返回借用引用；
//! - `cap_spawn_send.rs`：`BLOCK_ON | SPAWN_SEND` —— 跨线程（compio 为
//!   线程本地交错）spawn、`TrJoinHandle` 句柄抽象、JoinErr 传播；
//! - `cap_spawn_local.rs`：`SPAWN_LOCAL` —— `!Send` 的 `Rc` 任务（tokio 需
//!   `LocalSet` 上下文；compio 运行时线程本地，无需额外上下文）；
//! - `cap_delay.rs`：`DELAY` —— 时间驱动与 time driver 前提；
//! - `cap_spawn_blocking.rs`：`BLOCK_ON | SPAWN_BLOCKING` —— 阻塞线程池与
//!   异步侧共存；
//! - `cap_full.rs`：`FULL` —— 五种能力协同 + 后端自省（`tag()` / `about()`）；
//! - `cap_zero.rs`：`0` —— 零能力边界：`Runtime` 首先是类型标签，任何能力
//!   调用都是编译错误。
//!
//! 「不能做到什么」的编译期负向演示集中在
//! [`strict_mode_check`]（`compile_fail` 文档测试，`cargo test --doc` 验证，
//! 两种后端下均生效）。
//!
//! # 后端实例与互斥守卫
//!
//! 当前后端由 `demo-*` feature 决定，本 crate 对外统一暴露为
//! [`abs_art_bridge`]（模块），并直接把能力项再导出到 crate 根，供
//! doctest / 下游以 `abs_art_demo::*` 引用，与具体后端无关。

// 后端实例：demo-tokio / demo-compio 二选一（见 Cargo.toml 的 features）。
// 两个分支都是 `pub use` 外部 crate 的重命名导入，把「当前后端的 bridge」
// 统一暴露为 abs_art_bridge。
#[cfg(feature = "demo-tokio")]
pub use bridge_tokio as abs_art_bridge;
#[cfg(feature = "demo-compio")]
pub use bridge_compio as abs_art_bridge;

// 互斥守卫：两个演示组同时启用 → 响亮报错（而不是静默选一个）。
// bridge 内部对「backend 只能启用一个」还有一道 compile_error，这里先拦住。
#[cfg(all(feature = "demo-tokio", feature = "demo-compio"))]
compile_error!("abs_art-demo：demo-tokio 与 demo-compio 互斥，一次构建只能启用一个");

// 至少要启用一个演示组，否则上面的 abs_art_bridge 别名不存在。
#[cfg(not(any(feature = "demo-tokio", feature = "demo-compio")))]
compile_error!("abs_art-demo：必须启用 demo-tokio 或 demo-compio 之一（默认 demo-tokio）");

/// 能力项再导出：业务库对外暴露与后端无关的能力入口。
///
/// 这样 doctest / 下游代码可以统一写 `abs_art_demo::BLOCK_ON` 而不必关心
/// 当前启用的是哪个后端实例。
pub use abs_art_bridge::{
    BLOCK_ON, DELAY, FULL, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, Runtime, RuntimeTag,
    TrAsyncRuntime, TrBlockOn, TrDelay, TrJoinHandle, TrSpawnBlocking, TrSpawnLocal,
    TrSpawnSend,
};

/// 业务库声明的能力组合：只需要 `block_on` + `spawn_send` 两种能力。
///
/// 请求了未声明（或当前后端不支持）的能力会在编译期报错（Tag 严格模式）。
pub type CapRt = Runtime<{ BLOCK_ON | SPAWN_SEND }>;

/// 最小能力声明：只需要 `block_on` 一种能力。
///
/// 对应 `examples/{tokio,compio}_demo/cap_block_on.rs` 的 smoke test；这里
/// 作为业务库函数，展示 `TrBlockOn` 放松 `'static` 约束（提交 `19a6525`）
/// 带来的实际收益——两个后端都支持（tokio 的 `Handle::block_on` 与 compio
/// 的 `Runtime::block_on` 均接受非 `'static` 的 future）。
pub type BlockOnRt = Runtime<{ BLOCK_ON }>;

/// 示例业务函数：spawn 一个任务计算 `x * 2`，再 `block_on` 等待结果。
///
/// 该函数**没有任何泛型参数**——运行时能力通过 [`CapRt`] 的类型参数静态声明，
/// 编译期完成校验，运行期零开销。
pub fn double_via_runtime(x: i32) -> i32 {
    <CapRt as TrBlockOn<_>>::block_on(async move {
        let handle = <CapRt as TrSpawnSend<_>>::spawn(async move { x * 2 });
        handle.await.unwrap()
    })
}

/// 示例业务函数：`block_on` 一个**借用栈上数据**的 future（非 `'static`）。
///
/// 旧约束（`F: Future + 'static` 且 `F::Output: 'static`）下，`async` 块借用
/// 局部 `data` 无法通过编译；上一提交把 [`TrBlockOn`] 的约束放松为
/// `F: Future` 后，这类「只在本栈帧内同步等一个 async 结果」的代码得以成立。
///
/// 注意：本函数只通过 [`BlockOnRt`] 的能力声明使用抽象 trait，不感知后端；
/// 「必须处于运行时上下文内」是后端契约（tokio 需要 `Handle::current()`、
/// compio 需要 `with_current`），由集成方保证。
pub fn sum_stack_data() -> usize {
    let data = [1usize, 2, 3, 4];
    <BlockOnRt as TrBlockOn<_>>::block_on(async { data.iter().sum() })
}

/// 目的：验证业务函数在真实后端上运行正常。
///
/// 实施策略：由本 crate 选定的演示组（`demo-tokio` 或 `demo-compio`）创建
/// 对应运行时，在运行时上下文内调用 [`double_via_runtime`] 与
/// [`sum_stack_data`]。
///
/// 通过依据：`double_via_runtime(21) == 42`；`sum_stack_data() == 10`。
#[cfg(all(test, feature = "demo-tokio"))]
mod tests_tokio {
    use super::*;

    #[test]
    fn business_fn_works() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async { double_via_runtime(21) });
        assert_eq!(out, 42);
    }

    /// 目的：验证放松 `'static` 后的 `TrBlockOn` 在业务库层面可用。
    ///
    /// 实施策略：在 tokio 运行时上下文内调用 [`sum_stack_data`]（内部
    /// `block_on` 一个借用局部数据的 future）。
    ///
    /// 通过依据：返回 1 + 2 + 3 + 4 == 10；若 `'static` 约束未放松，
    /// 本测试将无法编译。
    #[test]
    fn borrow_block_on_works() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async { sum_stack_data() });
        assert_eq!(out, 10);
    }
}

/// 目的：与 `tests_tokio` 相同，但运行在 compio 后端上。
///
/// 实施策略：创建 compio 运行时（`Runtime::new()` 默认开启全部 driver），
/// 在 `rt.block_on` 上下文内调用业务函数。
///
/// 通过依据：`double_via_runtime(21) == 42`；`sum_stack_data() == 10`。
#[cfg(all(test, feature = "demo-compio"))]
mod tests_compio {
    use super::*;

    #[test]
    fn business_fn_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async { double_via_runtime(21) });
        assert_eq!(out, 42);
    }

    #[test]
    fn borrow_block_on_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async { sum_stack_data() });
        assert_eq!(out, 10);
    }
}

/// 编译期能力检查（Tag 严格模式）的负向演示集合。
///
/// 每一条 `compile_fail` 文档测试对应一句「**不能做到什么**」：能力声明
/// 没有覆盖到的操作，一律在**编译期**被拒绝，而不是运行期悄悄出错。
/// 正向演示（可以做到什么）见 `examples/{tokio,compio}_demo/` 下各 cap 的
/// smoke test。
///
/// 这些负向演示与后端无关：能力位掩码与 trait 约束定义在基础 crate
/// `abs_art`，两个后端实例行为一致。
pub mod strict_mode_check {
    /// 1. 只声明 `BLOCK_ON`，调用 `spawn`（需要 `SPAWN_SEND` 位）→ 编译错误。
    ///
    /// ```compile_fail
    /// use abs_art_demo::{BLOCK_ON, Runtime, TrSpawnSend};
    ///
    /// let _ = <Runtime<{ BLOCK_ON }> as TrSpawnSend<_>>::spawn(async { 1 });
    /// ```
    pub mod no_spawn_without_send_cap {}

    /// 2. 零能力 `Runtime<0>` 调用 `block_on` → 编译错误。
    ///
    /// ```compile_fail
    /// use abs_art_demo::{Runtime, TrBlockOn};
    ///
    /// let _ = <Runtime<0> as TrBlockOn<_>>::block_on(async { 1 });
    /// ```
    pub mod zero_caps_no_block_on {}

    /// 3. `spawn` 要求 future 是 `Send`：投递**捕获**了 `Rc` 的任务 → 编译错误。
    ///
    /// 注意：`Rc` 必须从外部被捕获（`async move` 把它移进 future），future
    /// 才会变成 `!Send`；若只写 `let _ = rc;`（通配符模式不移动），future
    /// 仍是 `Send`，可以编译。
    ///
    /// ```compile_fail
    /// use std::rc::Rc;
    /// use abs_art_demo::{BLOCK_ON, SPAWN_SEND, Runtime, TrSpawnSend};
    ///
    /// let rc = Rc::new(1); // !Send：被 async move 捕获后，future 不是 Send
    /// let _ = <Runtime<{ BLOCK_ON | SPAWN_SEND }> as TrSpawnSend<_>>::spawn(async move {
    ///     let _x = rc;
    ///     1
    /// });
    /// ```
    pub mod spawn_requires_send {}

    /// 4. `spawn` 要求 future 是 `'static`：借用局部数据 → 编译错误。
    ///
    /// 对照：[`crate::sum_stack_data`] 用 `block_on` 可以借用——`TrBlockOn`
    /// 的 `'static` 约束已放松（上一提交），但 `TrSpawnSend` **没有**放松：
    /// 任务要脱离当前栈帧运行，借用必然不成立。
    ///
    /// ```compile_fail
    /// use abs_art_demo::{BLOCK_ON, SPAWN_SEND, Runtime, TrSpawnSend};
    ///
    /// let data = vec![1, 2, 3];
    /// let _ = <Runtime<{ BLOCK_ON | SPAWN_SEND }> as TrSpawnSend<_>>::spawn(async {
    ///     data.iter().sum::<i32>() // data 是借用，非 'static
    /// });
    /// ```
    pub mod spawn_requires_static {}

    /// 5. 声明了 `BLOCK_ON | SPAWN_SEND`，调用 `spawn_local`（需要
    ///    `SPAWN_LOCAL` 位）→ 编译错误。
    ///
    /// ```compile_fail
    /// use abs_art_demo::{BLOCK_ON, SPAWN_SEND, Runtime, TrSpawnLocal};
    ///
    /// let _ = <Runtime<{ BLOCK_ON | SPAWN_SEND }> as TrSpawnLocal<_>>::spawn_local(async { 1 });
    /// ```
    pub mod no_spawn_local_without_local_cap {}

    /// 6. `tag()` 是 `Runtime<FULL>` 的固有方法（`impl Runtime<FULL>`），
    ///    不随 `CAPS` 泛化：零能力 `Runtime<0>` 没有它 → 编译错误。
    ///
    /// 自省请走 `TrAsyncRuntime::about()`（对所有 `CAPS` 实现，见
    /// `examples/{tokio,compio}_demo/cap_zero.rs` 的正向演示）。
    ///
    /// ```compile_fail
    /// use abs_art_demo::Runtime;
    ///
    /// let _ = Runtime::<0>::tag();
    /// ```
    pub mod tag_is_full_only {}
}
