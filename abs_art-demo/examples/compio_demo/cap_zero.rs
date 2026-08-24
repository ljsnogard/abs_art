//! # 设计意图
//!
//! 用**零能力** `Runtime<0>` 验证能力模型的边界（compio 演示组，与
//! `examples/tokio_demo/cap_zero.rs` 一一对应）：
//!
//! 1. **`Runtime` 首先是类型标签**：即使 `CAPS = 0`（一个能力位都没有），
//!    `Runtime<0>` 仍然是合法的零大小类型（ZST），可以 `current()` 取得、
//!    可以放进类型签名——「零能力」也是一种合法的能力声明；
//! 2. **最小权限原则的极端形态**：`Runtime<0>` 不实现任何能力 trait，
//!    调用任意能力（`block_on` / `spawn` / `delay` / …）都是编译错误——
//!    Tag 严格模式把「没用到的能力」在编译期就挡住；
//! 3. **固有方法 vs trait 方法的差异**：`tag()` 只定义在 `impl Runtime<FULL>`
//!    上（固有方法不随 `CAPS` 泛化），而 `TrAsyncRuntime::about()` 对所有
//!    `CAPS` 实现——自省能力与运行时能力是解耦的。
//!
//! 与 tokio 组唯一的不同：本文件断言 `about()` 报告的是 [`RuntimeTag::Compio`]，
//! 证明「零能力标签也能自省后端身份」这条性质在两个后端上一致成立。
//!
//! # 可以做到
//!
//! - `Runtime::<0>::current()` 取得零大小标签值；
//! - `<Runtime<0> as TrAsyncRuntime>::about()` 自省后端身份；
//! - 作为「占位 / 尚未决定能力」的类型出现在签名里，编译期零开销。
//!
//! # 不能做到
//!
//! - 调用任何能力 trait（`block_on` / `spawn` / `delay` / …）→ **编译错误**
//!   （见 [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo) 的
//!   `zero_caps_no_block_on`）；
//! - `Runtime::<0>::tag()` → 编译错误：`tag()` 是 `Runtime<FULL>` 的固有
//!   方法（见 `tag_is_full_only` 的 compile_fail）——自省请走
//!   `TrAsyncRuntime::about()`。

use bridge_compio::{Runtime, RuntimeTag, TrAsyncRuntime};

/// 零能力声明：`Runtime` 只是一个类型标签，不具备任何运行时能力。
type ZeroRt = Runtime<0>;

fn main() {
    // 零能力标签仍然是合法 ZST：current() 是 const fn，零运行期开销
    let rt: ZeroRt = ZeroRt::current();
    let _ = rt;

    // about() 通过 trait 对所有 CAPS 实现：零能力也能自省后端身份
    let about = <ZeroRt as TrAsyncRuntime>::about();
    assert_eq!(about, RuntimeTag::Compio);

    println!("compio cap_zero OK: about={about:?}");
}
