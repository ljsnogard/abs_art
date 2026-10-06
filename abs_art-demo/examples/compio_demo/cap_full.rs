//! # 设计意图
//!
//! 用**默认全能力** `Runtime<FULL>`（`Runtime` 不带参数时的默认值）验证
//! （compio 演示组，与 `examples/tokio_demo/cap_full.rs` 一一对应）：
//!
//! 1. **`FULL` 掩码 = 全部五个能力位**：`BLOCK_ON | DELAY | SPAWN_SEND |
//!    SPAWN_LOCAL | SPAWN_BLOCKING`，一个值同时拥有全部能力；
//! 2. **四种能力位 + 一个声明位 + 本地投递协同**：同一份业务代码里交替使用
//!    `spawn` / `delay` / `spawn_blocking`，本地投递经 `rt.spawn_local` 完成；
//! 3. **后端自省**：`rt.tag()`（固有方法）与 `rt.about()`（`TrAsyncRuntime`）
//!    报告当前后端身份——compio 组断言的是 `RuntimeTag::Compio`，与 tokio 组的
//!    `Tokio` 形成对照，证明自省机制真的能区分后端。
//!
//! # 值语义（v0.4）
//!
//! 能力方法全部收 `&self`，打在同一个运行时值上；自省也从「问类型」
//! （`Runtime::tag()`）变成「问这个值」（`value.tag()` / `value.about()`）。
//!
//! # 可以做到
//!
//! - 一个 `Runtime<FULL>` 值同时满足全部能力 trait；
//! - 在同一个 async 块中混用多种能力；
//! - 自省后端身份（`tag()` / `about()`）。
//!
//! # 不能做到
//!
//! - `FULL` 不提供后端**特有 API**（tokio 的 `sync::Mutex`、compio 的 IOCP
//!   事件、smol 的 `async_io` 设施等）——抽象层只承诺这几种能力，
//!   超出即不承诺；
//! - 能力在**运行期不能增减**：声明是编译期常量，`FULL` 与 `Runtime<0>`
//!   之间没有动态转换；
//! - **调度语义的后端差异**：tokio 组里 `block_on` 必须放进多线程运行时
//!   （`block_in_place` 限制），本地投递还要额外持有一条 `LocalSet`；
//!   compio 组没有这两条限制（运行时线程本地、队列归运行时自己所有，
//!   `run_until` 即 future 本身），因此本文件用一个 `rt.block_on` 就完成全部
//!   演示——「不能做到什么」随后端而变，这正是抽象层只承诺能力、不承诺调度
//!   细节的体现。
//!
//! # 与 tokio 组的另一处对照：本地投递的载体
//!
//! tokio 组的 `spawn_local` 需要额外的 `LocalSet` 与显式 `run_until` 驱动；
//! compio 的运行时本身就是线程本地的，`spawn` 与 `spawn_local` 是同一条队列、
//! 同一个入口，`run_until` 原样返回 `future`。写法一致，代价不同——值如实
//! 说出自己的前提。

use std::time::Duration;

use bridge_compio::{
    FULL, Runtime, RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay, TrLocalScope,
    TrSpawnBlocking, TrSpawnSend,
};

/// FULL 能力声明（默认值）：也可以直接写 `Runtime`，默认参数就是 FULL。
type FullRt = Runtime<FULL>;

/// 多能力业务函数：spawn + delay + spawn_blocking + 本地投递协同。
///
/// compio 下没有 `LocalSet` / `block_in_place` 的限制，一个函数就能用完
/// 全部能力（`block_on` 在 `main` 里作为外层驱动）。
async fn everything(rt: &FullRt) -> i32 {
    // 1) spawn：投递到本值抓住的运行时的当前工作队列
    let h = rt.spawn(async { 10 });
    let a = h.await.unwrap();

    // 2) delay：时间驱动
    rt.delay(Duration::from_millis(1)).await;

    // 3) spawn_blocking：阻塞线程
    let h = rt.spawn_blocking(|| 20);
    let b = h.await.unwrap();

    // 4) 本地投递：!Send 的 Rc 任务（compio 的队列归运行时，与 spawn 同一条）
    let c = {
        let rc = std::rc::Rc::new(12i32);
        let rc2 = rc.clone();
        let h = rt.spawn_local(async move { *rc2 });
        h.await.unwrap()
    };

    a + b + c
}

fn main() {
    // ---- 全能力协同：compio 一个 rt.block_on 即可（队列归运行时自己驱动）----
    let rt = compio::runtime::Runtime::new().unwrap();
    let out = rt.block_on(async {
        let value = FullRt::current();

        // ---- 自省：tag()（固有方法）与 about()（TrAsyncRuntime）----
        assert_eq!(value.tag(), RuntimeTag::Compio);
        assert_eq!(value.about(), RuntimeTag::Compio);

        // 在同一个值上聚合业务 future：`block_on` 由这份运行时的值方法提供
        value.block_on(everything(&value))
    });

    assert_eq!(out, 42, "spawn(10) + spawn_blocking(20) + Rc(12)");
    println!("compio cap_full OK: all_caps={out}, tag=Compio");
}
