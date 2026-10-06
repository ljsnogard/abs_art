//! # 设计意图
//!
//! 用**零能力** `Runtime<0>` 验证能力模型的边界：
//!
//! 1. **`Runtime` 首先是运行时值的类型**：即使 `CAPS = 0`（一个能力位都没有），
//!    `Runtime<0>` 仍然是合法的、可构造的运行时值，可以放进类型签名——
//!    「零能力」也是一种合法的能力声明；
//! 2. **最小权限原则的极端形态**：`Runtime<0>` 不实现任何能力 trait，
//!    调用任意能力（`block_on` / `spawn` / `delay` / …）都是编译错误——
//!    Tag 严格模式把「没用到的能力」在编译期就挡住；
//! 3. **自省与能力解耦**：`TrAsyncRuntime::about()` 对所有 `CAPS` 实现，
//!    因此零能力值仍能报告后端身份；而 `tag()` 也是全 `CAPS` 的固有方法。
//!
//! # 构造仍然需要上下文
//!
//! 零能力值同样是**值**：它由 `ZeroRt::current()` 在后端运行时上下文内构造，
//! 抓住一个 tokio 句柄，只是这个类型没有实现任何能力 trait。因此
//! 「零能力」= 「拿到了运行时，但承诺一件事都不做」。
//!
//! # 可以做到
//!
//! - `Runtime::<0>::current()` 取得零能力运行时值（需处于运行时上下文内）；
//! - `<Runtime<0> as TrAsyncRuntime>::about()` 自省后端身份；
//! - 作为「占位 / 尚未决定能力」的类型出现在签名里。
//!
//! # 不能做到
//!
//! - 调用任何能力 trait（`block_on` / `spawn` / `delay` / …）→ **编译错误**
//!   （见 [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo) 的
//!   `zero_caps_no_block_on`）；
//! - 取本地作用域：`local_scope()` 要求 `CAPS` 含 `SPAWN_LOCAL`，零能力值自然
//!   也拿不到（`local_scope_requires_declaration`）；
//! - 在没有任何运行时上下文的线程里构造值 → **运行期 panic**（不是编译错误）：
//!   `current()` 需要 `Handle::current()`，见 `strict_mode_check` 的
//!   `no_context_construction`。

use bridge_tokio::{RuntimeTag, TokioRuntime as Runtime, TrAsyncRuntime};

/// 零能力声明：这个值不承诺任何运行时能力。
type ZeroRt = Runtime<0>;

fn main() {
    // tokio 需要一个运行时上下文；零能力值也必须由上下文构造
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    let about = rt.block_on(async {
        // 零能力值仍然是合法的运行时值（构造需要上下文）
        let value: ZeroRt = ZeroRt::current();

        // about() 通过 trait 对所有 CAPS 实现：零能力也能自省后端身份
        value.about()
    });

    assert_eq!(about, RuntimeTag::Tokio);
    println!("cap_zero OK: about={about:?}");
}
