//! `abs_art-demo` 二进制：负责创建运行时、构造运行时**值**并调用业务库。
//!
//! 后端选择发生在 `Cargo.toml`（`demo-tokio` / `demo-compio` feature）：
//!
//! - 默认 `demo-tokio`：创建 tokio 运行时；
//! - `--no-default-features --features demo-compio`：创建 compio 运行时。
//!
//! 本文件是唯一允许感知后端的地方：创建哪个运行时的代码必须与所选后端一致；
//! 业务库（`src/lib.rs`）在两种后端下零改动。
//!
//! # 集成方的三步（v0.4 值语义）
//!
//! 1. **创建运行时**（后端特有：tokio 的 `Runtime::new()` / compio 的
//!    `Runtime::new()`）；
//! 2. **构造运行时值**：在后端上下文内构造声明了能力的
//!    [`abs_art_demo::CapRt`]（`current()` 需要上下文，见
//!    `strict_mode_check::no_context_construction`）；
//! 3. **把值交给业务函数**：`double_via_runtime(&value, 21)`。
//!
//! 第 2 步是 v0.4 新增的：业务函数不再自己「凭空」取得运行时，能力与环境前提
//! 一起由这个值承载。

/// tokio 演示组：用 tokio 运行时驱动业务库。
#[cfg(feature = "demo-tokio")]
fn main() {
    // 第 1 步：创建后端运行时（这里唯一感知 tokio 的地方）。
    let rt = tokio::runtime::Runtime::new().unwrap();

    // 第 2、3 步：在运行时上下文内构造值并交给业务函数。
    let out = rt.block_on(async {
        // 声明 `BLOCK_ON | SPAWN_SEND` 的运行时值；类型别名已固定 CAPS，
        // 因此这里不需要 turbofish。
        let value = abs_art_demo::CapRt::current();
        abs_art_demo::double_via_runtime(&value, 21)
    });

    assert_eq!(out, 42);
    println!("abs_art-demo (tokio backend) OK: {out}");
}

/// compio 演示组：用 compio 运行时驱动业务库。
///
/// compio 的 `Runtime::new()` 默认开启全部 driver（含 time），且运行时是
/// 线程本地的，因此这里不需要像 tokio 那样选 multi-thread / enable_all。
#[cfg(feature = "demo-compio")]
fn main() {
    // 第 1 步：创建后端运行时（这里唯一感知 compio 的地方）。
    let rt = compio::runtime::Runtime::new().unwrap();

    // 第 2、3 步：在运行时上下文内构造值并交给业务函数。
    let out = rt.block_on(async {
        let value = abs_art_demo::CapRt::current();
        abs_art_demo::double_via_runtime(&value, 21)
    });

    assert_eq!(out, 42);
    println!("abs_art-demo (compio backend) OK: {out}");
}
