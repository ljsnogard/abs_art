//! # 设计意图
//!
//! 用**本后端的完整能力集** `Runtime<{ CompioFull }>`（`CompioFull == 59`）验证
//! （compio 演示组，与 `examples/tokio_demo/cap_full.rs` 一一对应）：
//!
//! 1. **完整能力集 = compio 兑现得了的全部能力位**：`BLOCK_ON | DELAY |
//!    SPAWN_LOCAL | SPAWN_BLOCKING | CLOCK`（**不含** `SPAWN_SEND`），一个值同时
//!    声明了这些能力；
//! 2. **两件套协同**：值上做 `block_on` / `delay` / `spawn_blocking`，作用域上做
//!    `spawn_local` / `run_until`——计时从值取、投递从作用域取；
//! 3. **后端自省**：`rt.tag()`（固有方法）与 `rt.about()`（`TrAsyncRuntime`）
//!    报告当前后端身份——compio 组断言的是 `RuntimeTag::Compio`，与 tokio 组的
//!    `Tokio` 形成对照，证明自省机制真的能区分后端。
//!
//! # 这里必须写清的一件事：compio 上**声明** `SPAWN_SEND` 即静态失败
//!
//! 能力位只表达业务侧「声明要什么」，能不能给由后端决定。compio 没有跨线程
//! 全局工作队列，它把这条事实做成**比「少一个方法」更早**的编译期拒绝：
//! `abs_art-compio` 的 `Runtime<CAPS>` 在**类型定义**上要求
//! `[(); CAPS]: CompioCaps_`，而该断言只对不含 `SPAWN_SEND` 位的掩码实现。于是
//! **值一出现就报错**：
//!
//! ```text
//! // 59 | 4 == 63：构造点即 E0277，而不是等到调用 spawn 才说「没有这个方法」
//! Runtime::<{ CompioFull | SPAWN_SEND }>::current()
//!   → error[E0277]: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力
//! ```
//!
//! 所以本示例：
//!
//! - **不**调用 `rt.spawn(..)`——连声明那一位的类型都构造不出来；
//! - 需要「投多个任务」时改用 `scope.spawn_local(..)`（见 `cap_spawn_send.rs`
//!   的反向演示与 `abs_art_demo::local_three_tasks`）。
//!
//! 与之对照，tokio 组的 `cap_full` 里 `rt.spawn(..)` 是**可以**用的，因为它写的是
//! 具名 `TokioFull`（`63`）。同一个「完整能力集」概念，两个后端的**常量不同**——
//! 这正是抽象层如实表达后端差异的方式，也是本文件必须用 `CompioFull` 而不是裸名
//! `FULL` 的原因（裸 `FULL` 是默认后端的完整集，并集构建下等于本组的 `59`，
//! 写在这里虽然也编译得过，但会让读者误以为它表达了「所有后端」）。
//!
//! # 可以做到
//!
//! - 一个 `Runtime<{ CompioFull }>` 值同时满足 `TrBlockOn` / `TrDelay` / `TrTime` /
//!   `TrClock` / `TrSpawnBlocking` 与作用域侧能力；
//! - 在同一个 async 块中混用多种能力（值上的 + 作用域上的）；
//! - 自省后端身份（`tag()` / `about()`）。
//!
//! # 不能做到
//!
//! - 声明 `SPAWN_SEND` 位：**构造点即静态失败**（E0277，负向用例见
//!   [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo) 的
//!   `compio_rejects_spawn_send_at_construction`）；
//! - 完整能力集不提供后端**特有 API**（compio 的 IOCP 事件、tokio 的
//!   `sync::Mutex`、smol 的 `async_io` 设施等）——抽象层只承诺这几种能力；
//! - 能力在**运行期不能增减**：声明是编译期常量，`CompioFull` 与 `Runtime<0>`
//!   之间没有动态转换；
//! - 跨线程并行：本地投递只在本线程交错执行（见 `cap_spawn_send.rs`）。

use std::time::Duration;

use bridge_compio::{
    CompioFull, CompioRuntime as Runtime, RuntimeTag, TrAsyncRuntime, TrBlockOn, TrDelay,
    TrLocalScope, TrSpawnBlocking,
};

/// 能力声明：**compio 后端的完整能力集** `CompioFull`（`59`，**不含**
/// `SPAWN_SEND`）。
///
/// 写成 `Runtime::<{ CompioFull | SPAWN_SEND }>` 会在构造点静态失败，见文件头部。
type FullRt = Runtime<{ CompioFull }>;

/// 多能力业务函数（A 部分）：**不含** `spawn`——compio 声明不了 `SPAWN_SEND`
/// 那一位（构造点即静态失败），所以这里只用 `delay` 与 `spawn_blocking`。
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
