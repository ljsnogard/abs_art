//! # 设计意图
//!
//! 用**默认全能力** `Runtime<FULL>`（`Runtime` 不带参数时的默认值）验证：
//!
//! 1. **`FULL` 掩码 = 全部五个能力位**：`BLOCK_ON | DELAY | SPAWN_SEND |
//!    SPAWN_LOCAL | SPAWN_BLOCKING`，一个类型同时拥有全部能力；
//! 2. **五种能力协同**：同一份业务代码里交替使用 `spawn_send` / `delay` /
//!    `spawn_blocking` / `spawn_local` / `block_on`；
//! 3. **后端自省**：`Runtime::tag()`（FULL 专属固有方法）与
//!    `TrAsyncRuntime::about()`（对所有 CAPS 实现）都能报告当前后端身份，
//!    集成方可据此做运行时自省 / 断言。
//!
//! # 可以做到
//!
//! - 一个 `Runtime<FULL>` 同时满足全部五个能力 trait；
//! - 在同一个 async 块中混用多种能力；
//! - 自省后端身份（`tag()` / `about()`）。
//!
//! # 不能做到
//!
//! - `FULL` 不提供后端**特有 API**（tokio 的 `sync::Mutex`、compio 的 IOCP
//!   事件、smol 的 `async_io` 设施等）——抽象层只承诺五种能力，超出即不承诺；
//! - 能力在**运行期不能增减**：声明是编译期常量，`FULL` 与 `Runtime<0>` 之间
//!   没有动态转换；
//! - `spawn_local` 仍要求 `LocalSet` 上下文（与 `FULL` 与否无关，是 tokio
//!   后端的契约）——所以本示例把 spawn_local 单独放进 LocalSet 部分执行；
//! - `block_on` 仍要求多线程运行时上下文（`block_in_place` 限制）——所以
//!   本示例的多能力部分放在多线程运行时里执行。

use std::time::Duration;

use bridge_tokio::{
    FULL, Runtime, RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay, TrSpawnBlocking,
    TrSpawnLocal, TrSpawnSend,
};

/// FULL 能力声明（默认值）：也可以直接写 `Runtime`，默认参数就是 FULL。
type FullRt = Runtime<FULL>;

/// 多能力业务函数（A 部分）：spawn_send + delay + spawn_blocking。
///
/// 不需要 LocalSet，也不需要额外的 block_on 能力——外层 await 即可。
async fn everything_except_local() -> i32 {
    // 1) spawn_send：跨线程任务
    let h = <FullRt as TrSpawnSend<_>>::spawn(async { 10 });
    let a = h.await.unwrap();

    // 2) delay：时间驱动
    <FullRt as TrDelay>::delay(Duration::from_millis(1)).await;

    // 3) spawn_blocking：阻塞池
    let h = <FullRt as TrSpawnBlocking<_, i32>>::spawn_blocking(|| 20);
    let b = h.await.unwrap();

    a + b // 10 + 20
}

/// 本地业务函数（B 部分）：spawn_local 承载 !Send 的 Rc 任务。
///
/// 需要 LocalSet 上下文（由 main 的 `LocalSet::block_on` 提供）。
async fn local_part() -> i32 {
    let rc = std::rc::Rc::new(12i32);
    let rc2 = rc.clone();
    let h = <FullRt as TrSpawnLocal<_>>::spawn_local(async move { *rc2 });
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
        <FullRt as TrBlockOn<_>>::block_on(everything_except_local())
    });
    assert_eq!(out, 30, "spawn(10) + spawn_blocking(20)");

    // ---- B 部分：LocalSet（spawn_local 需要线程本地上下文）----
    let rt2 = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    let out2 = local.block_on(&rt2, async { local_part().await });
    assert_eq!(out2, 12, "Rc 任务返回值");

    println!("cap_full OK: multi_caps={out}, local={out2}, tag={:?}", Runtime::tag());
}
