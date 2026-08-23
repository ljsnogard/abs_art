//! `abs_art-demo` 二进制：负责创建运行时并调用业务库。
//!
//! 后端选择发生在 `Cargo.toml`（`abs_art-bridge` 的 `backend-*` feature），
//! 本文件不感知任何具体后端。

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt.block_on(async { abs_art_demo::double_via_runtime(21) });
    assert_eq!(out, 42);
    println!("abs_art-demo OK: {out}");
}
