//! # 设计意图
//!
//! 用**本后端的完整能力集** `Runtime<{ TokioFull }>`（`TokioFull == 63`）验证：
//!
//! 1. **完整能力集 = 全部六个能力位**：`BLOCK_ON | DELAY | SPAWN_SEND |
//!    SPAWN_LOCAL | SPAWN_BLOCKING | CLOCK`（`TokioFull` 是 tokio 后端自己的
//!    `FULL`），一个值同时拥有全部能力；
//! 2. **两件套协同**：同一份业务流程里交替使用值上的
//!    `spawn` / `delay` / `spawn_blocking` / `block_on`，以及作用域上的
//!    `spawn_local` / `run_until`；
//! 3. **后端自省**：`rt.tag()`（固有方法）与 `rt.about()`（`TrAsyncRuntime`）
//!    都能报告当前后端身份，集成方可据此做运行时自省 / 断言。
//!
//! # 为什么这里写 `TokioFull` 而不是裸名 `FULL`
//!
//! `abs_art-bridge` 的裸名 `FULL` 是**默认后端**的完整能力集。在 workspace 的
//! feature 并集构建下默认后端是 compio，裸 `FULL` 因此等于 compio 的 `59`
//! （**不含** `SPAWN_SEND`）——用它写 tokio 组的「全能力」会**静默少一位**，
//! `rt.spawn(..)` 直接不可用，而错误信息只会说「没有 `spawn` 方法」。具名的
//! `TokioFull` / `CompioFull` 随各自后端的 feature 存在，在并集构建下也精确，
//! 因此本 crate 的 [`abs_art_demo::FullRt`] 与 `current()` 一律取具名常量。
//!
//! # 为什么必须是两件套
//!
//! `TokioFull` 值同时具备六种能力，但**本地队列不在值里**：`SPAWN_LOCAL` 位只让
//! `Runtime::local_scope()` 可用，队列本身由交出的作用域值承载。于是本示例的
//! 结构是：
//!
//! ```text
//! value = FullRt::current()      // 运行时值：spawn / delay / spawn_blocking / block_on
//! scope = value.local_scope()    // 作用域：spawn_local / run_until
//! value.delay(..) 在 scope.run_until(..) 内部 —— 计时从值取、投递从作用域取
//! ```
//!
//! # 可以做到
//!
//! - 一个 `Runtime<{ TokioFull }>` 值同时满足全部能力 trait；
//! - 在同一个 async 块中混用多种能力（值上的 + 作用域上的）；
//! - 自省后端身份（`tag()` / `about()`）。
//!
//! # 不能做到
//!
//! - 完整能力集不提供后端**特有 API**（tokio 的 `sync::Mutex`、compio 的 IOCP
//!   事件、smol 的 `async_io` 设施等）——抽象层只承诺这几种能力，超出即不承诺；
//! - 能力在**运行期不能增减**：声明是编译期常量，`TokioFull` 与 `Runtime<0>`
//!   之间没有动态转换（`Runtime::retag` 只能换同后端的能力标签，不会改变运行期
//!   指向的对象）；
//! - **`SPAWN_LOCAL` 位与作用域缺一不可**：本地投递既要求声明（位），也要求真
//!   的取到作用域值——两者各管一半，见 `cap_spawn_local`；
//! - `block_on` 仍要求多线程运行时上下文（`block_in_place` 限制）——所以本示例
//!   放在多线程运行时里执行。

use std::time::Duration;

use bridge_tokio::{
    RuntimeTag, TokioFull, TokioRuntime as Runtime, TrAsyncRuntime, TrBlockOn, TrDelay,
    TrLocalScope, TrSpawnBlocking, TrSpawnSend,
};

/// 能力声明：**tokio 后端的完整能力集** `TokioFull`（`63`，六位全置）。
///
/// 刻意不用裸名 `FULL`：那是默认后端的完整能力集（并集构建下 = compio 的
/// `59`），会让本组静默缺掉 `SPAWN_SEND` 一位。详见文件头部说明。
type FullRt = Runtime<{ TokioFull }>;

/// 多能力业务函数（A 部分）：spawn + delay + spawn_blocking。
///
/// 三件都打在**运行时值**上，不需要作用域——外层 await 即可。
async fn everything_except_local_(rt: &FullRt) -> i32 {
    // 1) spawn：跨线程任务
    let h = rt.spawn(async { 10 });
    let a = h.await.unwrap();

    // 2) delay：时间驱动（计时属于运行时值）
    rt.delay(Duration::from_millis(1)).await;

    // 3) spawn_blocking：阻塞池
    let h = rt.spawn_blocking(|| 20);
    let b = h.await.unwrap();

    a + b // 10 + 20
}

/// 本地业务函数（B 部分）：在**作用域**上投递 `!Send` 的 `Rc` 任务。
///
/// 只依赖 `S: TrLocalScope`：本地投递与运行时值无关，只与队列有关。
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
    // ---- 全能力协同：多线程运行时（block_on 能力 + 多能力协同）----
    // 多线程是因为 TrBlockOn 的 tokio 实现基于 block_in_place（见 cap_block_on）
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all() // delay 需要 time driver
        .build()
        .unwrap();

    let (multi, local) = rt.block_on(async {
        let value = FullRt::current();

        // ---- 自省：tag()（固有方法）与 about()（TrAsyncRuntime）----
        assert_eq!(value.tag(), RuntimeTag::Tokio);
        assert_eq!(value.about(), RuntimeTag::Tokio);

        // 作用域：本地队列的持有者与驱动点（取得入口被 SPAWN_LOCAL 位门控）
        let scope = value.local_scope();

        // A 部分：值上的多能力协同（内部还用了 value.block_on）
        let multi = value.block_on(everything_except_local_(&value));

        // B 部分：两件套同框——计时从值取、投递与驱动从作用域取
        let local = scope
            .run_until(async {
                value.delay(Duration::from_millis(1)).await;
                local_part_(&scope).await
            })
            .await;

        (multi, local)
    });

    assert_eq!(multi, 30, "spawn(10) + spawn_blocking(20)");
    assert_eq!(local, 12, "Rc 本地任务返回值");
    println!("cap_full OK: multi_caps={multi}, local={local}, tag=Tokio");
}
