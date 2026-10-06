//! # 设计意图
//!
//! 用**最小能力声明** `Runtime<{ BLOCK_ON }>` 验证两件事：
//!
//! 1. **Tag 能力模型的最小形态**：`Runtime<CAPS>` 可以精确到「只要 `block_on`
//!    一种能力」，声明之外的能力（`spawn` / `delay` / …）一律编译期拒绝；
//! 2. **`TrBlockOn` 的 `'static` 约束放松**（上一提交 `19a6525` 把
//!    `F: Future + 'static` 且 `F::Output: 'static` 放松为 `F: Future`）：
//!    放松后 `block_on` 可以驱动**借用栈上数据**的 future、以及**返回借用引用**
//!    的 future——这正是本次 smoke test 最想用代码钉死的点。
//!
//! # 值语义（v0.4）
//!
//! 本示例演示「业务函数接收运行时**值**」的形状：`sum_stack_data(rt)` 等函数
//! 的形参是 `&BlockOnRt`，函数体里在**这个值**上调用 `rt.block_on(..)`。值由
//! `main`（集成方）在后端运行时上下文内用 `BlockOnRt::current()` 构造——
//! `block_on` 打在这个值抓住的 tokio 句柄上，而不是打在「当前线程恰好处于哪个
//! 运行时」这个隐含事实上。
//!
//! # 可以做到
//!
//! - `block_on` 一个捕获局部变量借用的 future（非 `'static` future）；
//! - `block_on` 一个 `Output` 是借用引用的 future（非 `'static` Output）；
//! - 在运行时上下文内同步等待 async 结果（tokio 后端的要求，见「不能做到 2」）。
//!
//! # 不能做到
//!
//! - `spawn` / `delay` 等未声明能力 → **编译错误**；`spawn_local` 同样不可用，
//!   原因有两层：`CAPS` 里没有 `SPAWN_LOCAL` 位（值因此不实现
//!   [`TrLocalScope`]），且即便有值也没有本地队列被驱动
//!   （负向演示见
//!   [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo) 的
//!   `compile_fail` 文档测试，例如 `no_spawn_without_send_cap` 与
//!   `spawn_local_requires_declaration`）；
//! - 在没有任何运行时上下文的线程里**构造值**（tokio 后端实现依赖
//!   `Handle::current()`，无上下文会 panic）——「必须处于运行时上下文内」是
//!   后端契约，由集成方（本文件的 `main`）保证；
//! - `spawn` 借用非 `'static` 数据：`TrSpawnSend` **没有**放松 `'static`
//!   约束（任务要脱离当前栈帧运行，借用必然不成立）——同一份"借用代码"，
//!   `block_on` 能过、`spawn` 不能过，这正是「可以做到什么」与「不能做到什么」
//!   的精确分界线。

use bridge_tokio::{BLOCK_ON, Runtime, TrBlockOn};

/// 能力声明：只请求 `block_on` 一种能力。
///
/// `CAPS` 只是编译期常量——掩码决定这个值实现了哪些能力 trait，从而决定哪些
/// 方法可调；值本身不向调用点穿透任何泛型参数。
type BlockOnRt = Runtime<{ BLOCK_ON }>;

/// 业务函数 A：`block_on` 一个**借用栈上数据**的 future。
///
/// `data` 是局部变量，`async` 块捕获的是对它的借用，future 类型不是 `'static`。
/// 旧约束（`F: Future + 'static`）下这段代码编译不过（`data does not live
/// long enough`）；上一提交把约束放松为 `F: Future` 后即可编译。
///
/// 这解决了实际痛点：很多一次性业务逻辑只是想「同步等一个 async 结果」，
/// 并不需要任务活得比当前栈帧更久，`'static` 要求纯属多余的负担。
fn sum_stack_data(rt: &BlockOnRt) -> usize {
    let data = [1usize, 2, 3, 4];
    // 借用 data 的 future：非 'static，直接在 block_on 里消费掉
    rt.block_on(async { data.iter().sum() })
}

/// 业务函数 B：`block_on` 的 future **返回一个借用引用**（Output 非 `'static`）。
///
/// 旧约束还要求 `<F as Future>::Output: 'static`，而这里 Output 是 `&[i32]`
/// （借用 `data`），必然不满足 `'static`——放松后可以，只要 `data` 在
/// `block_on` 返回之后仍然存活（本函数里确实如此）。
fn slice_then_sum(rt: &BlockOnRt) -> i32 {
    let data = [1i32, 2, 3];
    // Output = &[i32]，生命周期与 data 绑定；block_on 返回后 data 仍存活
    let slice = rt.block_on(async { data.as_slice() });
    slice.iter().sum::<i32>()
}

/// 业务函数 C：`block_on` 一个借用局部 `String` 的 future（方法调用即借用）。
fn str_len(rt: &BlockOnRt) -> usize {
    let s = String::from("hello");
    rt.block_on(async { s.len() })
}

fn main() {
    // 唯一感知后端的地方：创建 tokio 多线程运行时。
    // 用多线程（而非 current_thread）是因为 tokio 后端的 TrBlockOn 实现基于
    // block_in_place，而 block_in_place 不允许在 current_thread 运行时内使用。
    let rt = tokio::runtime::Builder::new_multi_thread()
        .build()
        .unwrap();

    // 外层 rt.block_on 提供「运行时上下文」，并在此构造抽象层的运行时**值**；
    // 内层才是值上的 TrBlockOn 调用。
    let (a, b, c) = rt.block_on(async {
        let value = BlockOnRt::current();
        let a = sum_stack_data(&value);
        let b = slice_then_sum(&value);
        let c = str_len(&value);
        (a, b, c)
    });

    assert_eq!(a, 10, "1+2+3+4");
    assert_eq!(b, 6, "1+2+3");
    assert_eq!(c, 5, "hello");
    println!("cap_block_on OK: sum_stack={a}, slice_sum={b}, str_len={c}");
}
