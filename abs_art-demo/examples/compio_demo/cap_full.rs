//! # 设计意图
//!
//! 用**默认全能力** `Runtime<FULL>`（`Runtime` 不带参数时的默认值）验证
//! （compio 演示组，与 `examples/tokio_demo/cap_full.rs` 一一对应）：
//!
//! 1. **`FULL` 掩码 = 全部五个能力位**：`BLOCK_ON | DELAY | SPAWN_SEND |
//!    SPAWN_LOCAL | SPAWN_BLOCKING`，一个值同时声明了全部能力；
//! 2. **两件套协同**：值上做 `block_on` / `delay` / `spawn_blocking`，作用域上做
//!    `spawn_local` / `run_until`——计时从值取、投递从作用域取；
//! 3. **后端自省**：`rt.tag()`（固有方法）与 `rt.about()`（`TrAsyncRuntime`）
//!    报告当前后端身份——compio 组断言的是 `RuntimeTag::Compio`，与 tokio 组的
//!    `Tokio` 形成对照，证明自省机制真的能区分后端。
//!
//! # 这里必须写清的一件事：`FULL` 在 compio 上**不含** `spawn`
//!
//! 能力位只表达业务侧「声明要什么」，能不能给由后端决定。compio 没有跨线程
//! 全局工作队列，因此它的运行时值**不实现** `TrSpawnSend`——即便位掩码写了
//! `SPAWN_SEND`（`FULL` 含该位），`rt.spawn(..)` 依然编译不过。所以本示例：
//!
//! - **不**调用 `rt.spawn(..)`（调用即编译错误）；
//! - 需要「投多个任务」时改用 `scope.spawn_local(..)`（见 `cap_spawn_send.rs`
//!   的反向演示与 `abs_art_demo::local_three_tasks`）。
//!
//! 与之对照，tokio 组的 `cap_full` 里 `rt.spawn(..)` 是**可以**用的。同一个
//! `FULL` 名字、同一个位掩码，两个后端能做的事不同——这正是抽象层如实表达的
//! 后端差异，也是本文件必须把「不能做到什么」写清楚的原因。
//!
//! # 可以做到
//!
//! - 一个 `Runtime<FULL>` 值同时满足 `TrBlockOn` / `TrDelay` / `TrTime` /
//!   `TrClock` / `TrSpawnBlocking` 与作用域侧能力；
//! - 在同一个 async 块中混用多种能力（值上的 + 作用域上的）；
//! - 自省后端身份（`tag()` / `about()`）。
//!
//! # 不能做到
//!
//! - `rt.spawn(..)`：compio 的运行时值不实现 `TrSpawnSend`（负向用例见
//!   [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo) 的
//!   `compio_runtime_has_no_spawn_send`）；
//! - `FULL` 不提供后端**特有 API**（compio 的 IOCP 事件、tokio 的
//!   `sync::Mutex`、smol 的 `async_io` 设施等）——抽象层只承诺这几种能力；
//! - 能力在**运行期不能增减**：声明是编译期常量，`FULL` 与 `Runtime<0>` 之间
//!   没有动态转换；
//! - 跨线程并行：本地投递只在本线程交错执行（见 `cap_spawn_send.rs`）。

use std::time::Duration;

use bridge_compio::{
    FULL, CompioRuntime as Runtime, RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay, TrLocalScope,
    TrSpawnBlocking,
};

/// FULL 能力声明（默认值）：也可以直接写 `Runtime`，默认参数就是 FULL。
type FullRt = Runtime<FULL>;

/// 多能力业务函数（A 部分）：**不含** `spawn`——compio 的运行时值不实现
/// `TrSpawnSend`，所以这里只用 `delay` 与 `spawn_blocking`。
///
/// 这也解释了为什么本文件与 tokio 组的 `cap_full` 不是逐字对应：tokio 组能多
/// 演示一项 `spawn`，compio 组不能。
async fn everything_except_spawn_(rt: &FullRt) -> i32 {
    // 1) delay：时间驱动（计时属于运行时值）
    rt.delay(Duration::from_millis(1)).await;

    // 2) spawn_blocking：阻塞池（三后端共有）
    let h = rt.spawn_blocking(|| 20);
    let b = h.await.unwrap();

    10 + b // 10 + 20
}

/// 本地业务函数（B 部分）：在**作用域**上投递 `!Send` 的 `Rc` 任务。
async fn local_part_<S>(scope: &S) -> i32
where
    S: TrLocalScope,
{
    let rc = std::rc::Rc::new(12i32);
    let rc2 = rc.clone();
    let h = scope.spawn_local(async move { *rc2 });
    h.await.unwrap()
}

fn main() {
    // ---- 全能力协同：compio 一个 rt.block_on 即可（队列归运行时自己驱动）----
    let rt = compio::runtime::Runtime::new().unwrap();

    let (multi, local) = rt.block_on(async {
        let value = FullRt::current();

        // ---- 自省：tag()（固有方法）与 about()（TrAsyncRuntime）----
        assert_eq!(value.tag(), RuntimeTag::Compio);
        assert_eq!(value.about(), RuntimeTag::Compio);

        // 作用域：本地投递的宿主与驱动点（入口被 SPAWN_LOCAL 位门控）
        let scope = value.local_scope();

        // A 部分：值上的多能力协同（内部用 value.block_on 聚合）
        let multi = value.block_on(everything_except_spawn_(&value));

        // B 部分：两件套同框——计时从值取、投递与驱动从作用域取
        let local = scope
            .run_until(async {
                value.delay(Duration::from_millis(1)).await;
                local_part_(&scope).await
            })
            .await;

        (multi, local)
    });

    assert_eq!(multi, 30, "10 + spawn_blocking(20)");
    assert_eq!(local, 12, "Rc 本地任务返回值");
    println!("compio cap_full OK: all_caps={multi}, local={local}, tag=Compio");
}
