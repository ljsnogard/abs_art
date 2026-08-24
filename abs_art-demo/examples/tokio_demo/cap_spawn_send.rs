//! # 设计意图
//!
//! 用**组合能力** `Runtime<{ BLOCK_ON | SPAWN_SEND }>` 验证：
//!
//! 1. **能力按位组合**：`BLOCK_ON | SPAWN_SEND` 是一个掩码，声明它 = 同时获得
//!    `block_on` 与 `spawn` 两种能力（与业务库 `abs_art_demo::CapRt` 完全同构）；
//! 2. **`TrJoinHandle` 的句柄抽象**：业务代码可以写出只依赖抽象 trait 的泛型
//!    工具函数（`join_abstract`），完全不感知 tokio / compio / smol 的具体句柄
//!    类型——这是「抽象层真的可落地」的关键证明；
//! 3. **任务 panic 的错误传播**：`JoinErr` 通过句柄的关联类型 `H::JoinErr`
//!    （`core::error::Error`）传给调用方。
//!
//! # 可以做到
//!
//! - `spawn` 一个 `Send + 'static` 的 future 到全局工作队列（多线程并行）；
//! - `await` 返回的 `JoinHandle` 取回 `Result<T, JoinErr>`；
//! - 多个任务并发执行后聚合结果；
//! - 任务 panic 时错误正常传播（`is_err()`），不会吞掉也不会上抛到进程。
//!
//! # 不能做到
//!
//! - `spawn` **非 `Send`** 的 future（如捕获 `Rc`）→ 编译错误（`F: Send` 约束，
//!   见 [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo) 的
//!   `spawn_requires_send`）；
//! - `spawn` **借用非 `'static`** 数据的 future → 编译错误（`F: 'static` 约束，
//!   对照 `cap_block_on` 里 `block_on` 可以借用——`TrSpawnSend` 没有放松
//!   `'static`，见 `spawn_requires_static`）；
//! - `spawn_local`（需要 `SPAWN_LOCAL` 位）→ 编译错误（见
//!   `no_spawn_local_without_local_cap`）；
//! - 句柄抽象只覆盖「等待/取结果」，不提供后端特有操作（如 tokio 的
//!   `abort` 之外的取消语义）——能力边界之外的东西不在抽象层承诺内。

use core::future::Future;

use bridge_tokio::{BLOCK_ON, SPAWN_SEND, Runtime, TrBlockOn, TrJoinHandle, TrSpawnSend};

/// 能力声明：`block_on` + `spawn_send`（与业务库 `CapRt` 同构的能力组合）。
type SendRt = Runtime<{ BLOCK_ON | SPAWN_SEND }>;

/// 泛型工具函数：等待**任意后端**的 JoinHandle。
///
/// 只依赖抽象 trait `TrJoinHandle<T>`（它的 supertrait 保证 `H` 是一个
/// `Future<Output = Result<T, H::JoinErr>>`），不感知任何后端句柄类型。
/// 具体句柄类型（`abs_art_tokio::JoinHandle` 等）只在编译期单态化时出现。
async fn join_abstract<H, T>(handle: H) -> Result<T, H::JoinErr>
where
    H: TrJoinHandle<T> + Future<Output = Result<T, H::JoinErr>>,
{
    handle.await
}

/// 业务函数：spawn 三个任务并发计算，再聚合结果。
///
/// 三个任务都投递到全局工作队列，由运行时的多个 worker 线程并行执行；
/// 返回值通过抽象的 `join_abstract` 取回，调用点没有任何后端类型泄漏。
async fn concurrent_sum(x: i32) -> i32 {
    let h1 = <SendRt as TrSpawnSend<_>>::spawn(async move { x });
    let h2 = <SendRt as TrSpawnSend<_>>::spawn(async move { x * 2 });
    let h3 = <SendRt as TrSpawnSend<_>>::spawn(async move { x * 3 });
    let a = join_abstract(h1).await.unwrap();
    let b = join_abstract(h2).await.unwrap();
    let c = join_abstract(h3).await.unwrap();
    a + b + c
}

/// 业务函数：任务内部 panic 时，错误通过 `JoinErr` 传播给 await 方。
async fn panic_propagates() -> bool {
    async fn boom() -> i32 {
        panic!("任务爆炸");
    }
    let h = <SendRt as TrSpawnSend<_>>::spawn(boom());
    // JoinErr 是 core::error::Error：await 拿到的是 Result，而不是直接抛给进程
    join_abstract(h).await.is_err()
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .build()
        .unwrap();

    let (sum, panicked) = rt.block_on(async {
        // 外层 tokio 上下文内再 block_on 聚合 future（块内执行 spawn 等操作）
        let sum = <SendRt as TrBlockOn<_>>::block_on(concurrent_sum(7));
        let panicked = <SendRt as TrBlockOn<_>>::block_on(panic_propagates());
        (sum, panicked)
    });

    assert_eq!(sum, 42, "7*1 + 7*2 + 7*3");
    assert!(panicked, "panic 任务必须通过 JoinErr 传播");
    println!("cap_spawn_send OK: sum={sum}, join_err_propagates={panicked}");
}
