//! `abs_art-demo` 二进制：负责创建运行时并调用业务库。
//!
//! 后端选择发生在 `Cargo.toml`（`demo-tokio` / `demo-compio` feature）：
//!
//! - 默认 `demo-tokio`：创建 tokio 运行时；
//! - `--no-default-features --features demo-compio`：创建 compio 运行时。
//!
//! 本文件是唯一允许感知后端的地方：创建哪个运行时的代码必须与所选后端
//! 一致；业务库（`src/lib.rs`）在两种后端下零改动。

/// tokio 演示组：用 tokio 运行时驱动业务库。
#[cfg(feature = "demo-tokio")]
fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt.block_on(async { abs_art_demo::double_via_runtime(21) });
    assert_eq!(out, 42);
    println!("abs_art-demo (tokio backend) OK: {out}");
}

/// compio 演示组：用 compio 运行时驱动业务库。
///
/// compio 的 `Runtime::new()` 默认开启全部 driver（含 time），且运行时是
/// 线程本地的，因此这里不需要像 tokio 那样选 multi-thread / enable_all。
#[cfg(feature = "demo-compio")]
fn main() {
    let rt = compio::runtime::Runtime::new().unwrap();
    let out = rt.block_on(async { abs_art_demo::double_via_runtime(21) });
    assert_eq!(out, 42);
    println!("abs_art-demo (compio backend) OK: {out}");
}
