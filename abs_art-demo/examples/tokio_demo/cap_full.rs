//! # 设计意图
//!
//! 用**默认全能力** `Runtime<FULL>`（`Runtime` 不带参数时的默认值）验证：
//!
//! 1. **`FULL` 掩码 = 全部五个能力位**：`BLOCK_ON | DELAY | SPAWN_SEND |
//!    SPAWN_LOCAL | SPAWN_BLOCKING`，一个类型同时拥有全部能力；
//! 2. **四种能力位 + 一个声明位 + 一个本地作用域协同**：同一份业务代码里交替使用 `spawn_send` /
//!    `delay` / `spawn_blocking` / `block_on`，本地投递则经[值](LocalScope)完成；
//! 3. **后端自省**：`Runtime::tag()`（FULL 专属固有方法）与
//!    `TrAsyncRuntime::about()`（对所有 CAPS 实现）都能报告当前后端身份，
//!    集成方可据此做运行时自省 / 断言。
//!
//! # 可以做到
//!
//! - 一个 `Runtime<FULL>` 同时满足全部能力 trait；
//! - 在同一个 async 块中混用多种能力；
//! - 自省后端身份（`tag()` / `about()`）。
//!
//! # 不能做到
//!
//! - `FULL` 不提供后端**特有 API**（tokio 的 `sync::Mutex`、compio 的 IOCP
//!   事件、smol 的 `async_io` 设施等）——抽象层只承诺这几种能力，超出即不承诺；
//! - 能力在**运行期不能增减**：声明是编译期常量，`FULL` 与 `Runtime<0>` 之间
//!   没有动态转换；
//! - **`SPAWN_LOCAL` 位只负责声明，不负责调用**：本地投递的调用点在值
//!   （[`LocalScope`]）上，位只决定能不能经 `Runtime::local_scope()` 取得它。
//!   哪怕声明了 `FULL`，不持有作用域值也投递不了本地任务——环境前提由值承载；
//! - `block_on` 仍要求多线程运行时上下文（`block_in_place` 限制）——所以
//!   本示例的多能力部分放在多线程运行时里执行。

use std::time::Duration;

use bridge_tokio::{
    FULL, LocalScope, Runtime, RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay,
    TrLocalScope, TrSpawnBlocking, TrSpawnSend,
};

/// FULL 能力声明（默认值）：也可以直接写 `Runtime`，默认参数就是 FULL。
type FullRt = Runtime<FULL>;

/// 多能力业务函数（A 部分）：spawn_send + delay + spawn_blocking。
///
/// 不需要本地作用域，也不需要额外的 block_on 能力——外层 await 即可。
async fn everything_except_local() -> i32 {
    // 1) spawn_send：跨线程任务
    let h = <FullRt as TrSpawnSend>::spawn(async { 10 });
    let a = h.await.unwrap();

    // 2) delay：时间驱动
    <FullRt as TrDelay>::delay(Duration::from_millis(1)).await;

    // 3) spawn_blocking：阻塞池
    let h = <FullRt as TrSpawnBlocking>::spawn_blocking(|| 20);
    let b = h.await.unwrap();

    a + b // 10 + 20
}

/// 本地业务函数（B 部分）：经本地作用域投递 `!Send` 的 `Rc` 任务。
///
/// 作用域由 `main` 创建并驱动——**投递点与驱动点都在同一个值上**，
/// 这就是「本地队列成为显式值」的直接体现。
async fn local_part(scope: &LocalScope) -> i32 {
    let rc = std::rc::Rc::new(12i32);
    let rc2 = rc.clone();
    let h = scope.spawn_local(async move { *rc2 });
    h.await.unwrap()
}

fn main() {
    // ---- 自省：tag()（FULL 专属固有方法）与 about()（trait，所有 CAPS）----
    assert_eq!(Runtime::tag(), RuntimeTag::Tokio);
    assert_eq!(<FullRt as TrAsyncRuntime>::about(), RuntimeTag::Tokio);

    // ---- A 部分：多线程运行时（block_on 能力 + 多能力协同）----
    // 多线程是因为 TrBlockOn 的 tokio 实现基于 block_in_place（见 cap_block_on）
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all() // delay 需要 time driver
        .build()
        .unwrap();
    let out = rt.block_on(async {
        <FullRt as TrBlockOn>::block_on(everything_except_local())
    });
    assert_eq!(out, 30, "spawn(10) + spawn_blocking(20)");

    // ---- B 部分：本地作用域（tokio 的本地队列由 LocalSet 持有，由 run_until 驱动）----
    let rt2 = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let scope = LocalScope::new();
    let out2 = rt2.block_on(scope.run_until(local_part(&scope)));
    assert_eq!(out2, 12, "Rc 任务返回值");

    println!("cap_full OK: multi_caps={out}, local={out2}, tag={:?}", Runtime::tag());
}
