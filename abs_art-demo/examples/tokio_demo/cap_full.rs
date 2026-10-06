//! # 设计意图
//!
//! 用**默认全能力** `Runtime<FULL>`（`Runtime` 不带参数时的默认值）验证：
//!
//! 1. **`FULL` 掩码 = 全部五个能力位**：`BLOCK_ON | DELAY | SPAWN_SEND |
//!    SPAWN_LOCAL | SPAWN_BLOCKING`，一个值同时拥有全部能力；
//! 2. **四种能力位 + 一个声明位 + 本地投递协同**：同一份业务代码里交替使用
//!    `spawn` / `delay` / `spawn_blocking` / `block_on`，本地投递经
//!    `rt.spawn_local` + `rt.run_until` 完成（本地队列归**这个值**所有）；
//! 3. **后端自省**：`rt.tag()`（固有方法）与 `rt.about()`（`TrAsyncRuntime`）
//!    都能报告当前后端身份，集成方可据此做运行时自省 / 断言。
//!
//! # 值语义（v0.4）
//!
//! `FULL` 现在是「这个**值**具备全部五种能力」：能力方法全部收 `&self`，
//! 打在同一个值上。自省也不再是 `Runtime::tag()` 这种「问类型」的形式，而是
//! `value.tag()` / `value.about()`——问的是**这个值**指向哪个运行时。
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
//!   事件、smol 的 `async_io` 设施等）——抽象层只承诺这几种能力，超出即不承诺；
//! - 能力在**运行期不能增减**：声明是编译期常量，`FULL` 与 `Runtime<0>` 之间
//!   没有动态转换（`Runtime::retag` 只能换同后端的能力标签，不会改变运行期
//!   指向的对象）；
//! - **`SPAWN_LOCAL` 位与值缺一不可**：本地投递既要求声明（位），也要求手上
//!   这个值真的持有并能驱动一条本地队列——两者各管一半，见 `cap_spawn_local`；
//! - `block_on` 仍要求多线程运行时上下文（`block_in_place` 限制）——所以
//!   本示例的多能力部分放在多线程运行时里执行。

use std::time::Duration;

use bridge_tokio::{
    FULL, Runtime, RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay, TrLocalScope,
    TrSpawnBlocking, TrSpawnSend,
};

/// FULL 能力声明（默认值）：也可以直接写 `Runtime`，默认参数就是 FULL。
type FullRt = Runtime<FULL>;

/// 多能力业务函数（A 部分）：spawn + delay + spawn_blocking。
///
/// 不需要本地投递，也不需要额外的 `block_on`——外层 await 即可。
async fn everything_except_local(rt: &FullRt) -> i32 {
    // 1) spawn：跨线程任务
    let h = rt.spawn(async { 10 });
    let a = h.await.unwrap();

    // 2) delay：时间驱动
    rt.delay(Duration::from_millis(1)).await;

    // 3) spawn_blocking：阻塞池
    let h = rt.spawn_blocking(|| 20);
    let b = h.await.unwrap();

    a + b // 10 + 20
}

/// 本地业务函数（B 部分）：在这个值上投递 `!Send` 的 `Rc` 任务。
///
/// **投递点与驱动点都在同一个值上**，这就是「本地队列并回运行时值」的直接
/// 体现：不再需要额外的作用域参数。
async fn local_part<R: TrLocalScope>(rt: &R) -> i32 {
    let rc = std::rc::Rc::new(12i32);
    let rc2 = rc.clone();
    let h = rt.spawn_local(async move { *rc2 });
    h.await.unwrap()
}

fn main() {
    // ---- A 部分：多线程运行时（block_on 能力 + 多能力协同）----
    // 多线程是因为 TrBlockOn 的 tokio 实现基于 block_in_place（见 cap_block_on）
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all() // delay 需要 time driver
        .build()
        .unwrap();
    let out = rt.block_on(async {
        let value = FullRt::current();

        // ---- 自省：tag()（固有方法）与 about()（TrAsyncRuntime）----
        assert_eq!(value.tag(), RuntimeTag::Tokio);
        assert_eq!(value.about(), RuntimeTag::Tokio);

        value.block_on(everything_except_local(&value))
    });
    assert_eq!(out, 30, "spawn(10) + spawn_blocking(20)");

    // ---- B 部分：本地投递（tokio 的本地队列由本值持有的 LocalSet 驱动）----
    let rt2 = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let out2 = rt2.block_on(async {
        let value = FullRt::current();
        value.run_until(local_part(&value)).await
    });
    assert_eq!(out2, 12, "Rc 任务返回值");

    println!("cap_full OK: multi_caps={out}, local={out2}, tag=Tokio");
}
