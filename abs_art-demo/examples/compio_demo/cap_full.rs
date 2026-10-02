//! # 设计意图
//!
//! 用**默认全能力** `Runtime<FULL>`（`Runtime` 不带参数时的默认值）验证
//! （compio 演示组，与 `examples/tokio_demo/cap_full.rs` 一一对应）：
//!
//! 1. **`FULL` 掩码 = 全部五个能力位**：`BLOCK_ON | DELAY | SPAWN_SEND |
//!    SPAWN_LOCAL | SPAWN_BLOCKING`，一个类型同时拥有全部能力；
//! 2. **四种能力位 + 一个声明位 + 一个本地作用域协同**：同一份业务代码里交替使用 `spawn_send` /
//!    `delay` / `spawn_blocking` / `block_on`，本地投递经[值](LocalScope)完成；
//! 3. **后端自省**：`Runtime::tag()`（FULL 专属固有方法）与
//!    `TrAsyncRuntime::about()`（对所有 CAPS 实现）报告当前后端身份——
//!    compio 组断言的是 `RuntimeTag::Compio`，与 tokio 组的 `Tokio` 形成
//!    对照，证明自省机制真的能区分后端。
//!
//! # 可以做到
//!
//! - 一个 `Runtime<FULL>` 同时满足全部能力 trait；
//! - 在同一个 async 块中混用多种能力；
//! - 自省后端身份（`tag()` / `about()`）。
//!
//! # 不能做到
//!
//! - `FULL` 不提供后端**特有 API**（tokio 的 `sync::Mutex`、compio 的
//!   IOCP 事件、smol 的 `async_io` 设施等）——抽象层只承诺这几种能力，
//!   超出即不承诺；
//! - 能力在**运行期不能增减**：声明是编译期常量，`FULL` 与 `Runtime<0>`
//!   之间没有动态转换；
//! - **调度语义的后端差异**：tokio 组里 `block_on` 必须放进多线程运行时
//!   （`block_in_place` 限制），本地队列还要额外提供并驱动一个 `LocalSet`；
//!   compio 组没有这两条限制（运行时线程本地、队列归运行时自己所有），
//!   因此本文件用一个 `rt.block_on` 就完成全部演示——「不能做到什么」随后端
//!   而变，这正是抽象层只承诺能力、不承诺调度细节的体现。
//!
//! # 作用域在这里为什么是零大小的
//!
//! [`LocalScope`] 在三个后端上**同名但内容不同**：tokio 版本装着一个 `LocalSet`，
//! smol 版本装着一条 `LocalExecutor`，而 compio 版本**什么都不装**——因为 compio
//! 的运行时自己就持有并驱动那条线程本地队列，调用方无需提供任何东西。空类型不是
//! 占位符，而是如实表达这一点。

use std::time::Duration;

use bridge_compio::{
    FULL, LocalScope, Runtime, RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay,
    TrLocalScope, TrSpawnBlocking, TrSpawnSend,
};

/// FULL 能力声明（默认值）：也可以直接写 `Runtime`，默认参数就是 FULL。
type FullRt = Runtime<FULL>;

/// 多能力业务函数：spawn_send + delay + spawn_blocking + 本地投递协同。
///
/// compio 下没有 `LocalSet` / `block_in_place` 的限制，一个函数就能用完
/// 全部能力（`block_on` 在 main 里作为外层驱动）。
async fn everything(scope: &LocalScope) -> i32 {
    // 1) spawn_send：投递到当前运行时工作队列
    let h = <FullRt as TrSpawnSend>::spawn(async { 10 });
    let a = h.await.unwrap();

    // 2) delay：时间驱动
    <FullRt as TrDelay>::delay(Duration::from_millis(1)).await;

    // 3) spawn_blocking：阻塞线程
    let h = <FullRt as TrSpawnBlocking>::spawn_blocking(|| 20);
    let b = h.await.unwrap();

    // 4) 本地投递：!Send 的 Rc 任务（compio 的队列归运行时，作用域为空）
    let c = {
        let rc = std::rc::Rc::new(12i32);
        let rc2 = rc.clone();
        let h = scope.spawn_local(async move { *rc2 });
        h.await.unwrap()
    };

    a + b + c
}

fn main() {
    // ---- 自省：tag()（FULL 专属固有方法）与 about()（trait，所有 CAPS）----
    assert_eq!(Runtime::tag(), RuntimeTag::Compio);
    assert_eq!(<FullRt as TrAsyncRuntime>::about(), RuntimeTag::Compio);

    // ---- 全能力协同：compio 一个 rt.block_on 即可（队列归运行时自己驱动）----
    let rt = compio::runtime::Runtime::new().unwrap();
    let scope = LocalScope::new();
    let out = rt.block_on(scope.run_until(async {
        // 外层 compio 上下文内 block_on 聚合 future（块内完成全部能力）
        <FullRt as TrBlockOn>::block_on(everything(&scope))
    }));

    assert_eq!(out, 42, "spawn(10) + spawn_blocking(20) + Rc(12)");
    println!(
        "compio cap_full OK: all_caps={out}, tag={:?}",
        Runtime::tag()
    );
}
