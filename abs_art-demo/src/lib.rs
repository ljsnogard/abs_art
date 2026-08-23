//! `abs_art-demo`：演示「业务库零泛型穿透 + 二进制通过 Cargo.toml 选后端」。
//!
//! - [`lib.rs`](crate)（业务库）：只依赖 [`abs_art_bridge`]，通过能力标签
//!   `Runtime::<{ BLOCK_ON | SPAWN_SEND }>` 声明所需能力，**没有任何泛型
//!   参数穿透**；
//! - `main.rs`（二进制）：负责创建运行时并调用业务库——它是唯一允许
//!   感知后端的地方（创建哪个运行时的代码必须与所选后端一致）。
//!
//! 切换后端 = 改本 crate `Cargo.toml` 里的 `backend-*` feature，并按需调整
//! `main.rs` 的运行时创建；**业务库（本文件）零改动**。

use abs_art_bridge::{BLOCK_ON, SPAWN_SEND, Runtime, TrBlockOn, TrSpawnSend};

/// 业务库声明的能力组合：只需要 `block_on` + `spawn_send` 两种能力。
///
/// 请求了未声明（或当前后端不支持）的能力会在编译期报错（Tag 严格模式）。
pub type CapRt = Runtime<{ BLOCK_ON | SPAWN_SEND }>;

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

/// 目的：验证业务函数在真实后端上运行正常。
///
/// 实施策略：由本 crate 的 `Cargo.toml` 选定的后端（默认 `backend-tokio`）
/// 提供运行时上下文，调用 [`double_via_runtime`]。
///
/// 通过依据：返回 21 * 2 == 42。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_fn_works() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async { double_via_runtime(21) });
        assert_eq!(out, 42);
    }
}

/// 编译期能力检查（Tag 严格模式）的负向演示：
/// 只声明了 `BLOCK_ON`，调用 `spawn`（需要 `SPAWN_SEND`）→ 编译错误。
///
/// ```compile_fail
/// use abs_art_bridge::{BLOCK_ON, Runtime, TrSpawnSend};
///
/// let _ = <Runtime<{ BLOCK_ON }> as TrSpawnSend<_>>::spawn(async { 1 });
/// ```
pub mod strict_mode_check {}
