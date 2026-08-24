//! # 设计意图
//!
//! 用**组合能力** `Runtime<{ BLOCK_ON | SPAWN_SEND }>` 验证（compio 演示组，
//! 与 `examples/tokio_demo/cap_spawn_send.rs` 一一对应）：
//!
//! 1. **能力按位组合**：`BLOCK_ON | SPAWN_SEND` 是一个掩码，声明它 = 同时获得
//!    `block_on` 与 `spawn` 两种能力；
//! 2. **`TrJoinHandle` 的句柄抽象**：业务代码可以写出只依赖抽象 trait 的泛型
//!    工具函数（`join_abstract`），完全不感知 tokio / compio / smol 的具体
//!    句柄类型——compio 组的句柄是 `compio::runtime::JoinHandle` 的薄包装，
//!    但本文件的代码与 tokio 组**逐字相同**，正是「抽象层真的可落地」的证明；
//! 3. **任务 panic 的错误传播**：`JoinErr` 通过句柄的关联类型 `H::JoinErr`
//!    （`core::error::Error`）传给调用方（compio 的 `JoinError::Panicked`）。
//!
//! # 可以做到
//!
//! - `spawn` 一个 `Send + 'static` 的 future 到当前 compio 运行时的工作队列；
//! - `await` 返回的 `JoinHandle` 取回 `Result<T, JoinErr>`；
//! - 多个任务投递后交错执行、聚合结果；
//! - 任务 panic 时错误正常传播（`is_err()`），不会吞掉也不会上抛到进程。
//!
//! # 不能做到
//!
//! - **真正的跨线程并行**：compio 运行时是**线程本地**的（`!Send`，不能跨
//!   线程发送），`spawn` 的任务都在当前线程的运行时上交错执行，不会像 tokio
//!   那样分散到多个 worker 核——能力声明与句柄抽象两者一致，但底层调度
//!   语义不同，这是抽象层不承诺的部分；
//! - `spawn` **非 `Send`** 的 future（如捕获 `Rc`）→ 编译错误（`F: Send`
//!   约束，见 [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo)
//!   的 `spawn_requires_send`）——注意：compio 原生 `rt.spawn` 不要求 `Send`，
//!   但抽象层的 `TrSpawnSend` 契约要求 `Send`，两个后端一致；
//! - `spawn` **借用非 `'static`** 数据的 future → 编译错误（对照 `cap_block_on`
//!   里 `block_on` 可以借用，见 `spawn_requires_static`）；
//! - `spawn_local`（需要 `SPAWN_LOCAL` 位）→ 编译错误（见
//!   `no_spawn_local_without_local_cap`）。

use core::future::Future;

use bridge_compio::{BLOCK_ON, SPAWN_SEND, Runtime, TrBlockOn, TrJoinHandle, TrSpawnSend};

/// 能力声明：`block_on` + `spawn_send`。
type SendRt = Runtime<{ BLOCK_ON | SPAWN_SEND }>;

/// 泛型工具函数：等待**任意后端**的 JoinHandle。
///
/// 只依赖抽象 trait `TrJoinHandle<T>`（它的 supertrait 保证 `H` 是一个
/// `Future<Output = Result<T, H::JoinErr>>`），不感知任何后端句柄类型。
/// 具体句柄类型（`abs_art_compio::JoinHandle` 等）只在编译期单态化时出现。
async fn join_abstract<H, T>(handle: H) -> Result<T, H::JoinErr>
where
    H: TrJoinHandle<T> + Future<Output = Result<T, H::JoinErr>>,
{
    handle.await
}

/// 业务函数：spawn 三个任务并发计算，再聚合结果。
///
/// compio 的任务在同一个线程本地运行时上交错执行；返回值通过抽象的
/// `join_abstract` 取回，调用点没有任何后端类型泄漏。
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
    // compio 的 JoinError::Panicked 同样会被捕获为 Err，而不是炸掉进程
    join_abstract(h).await.is_err()
}

fn main() {
    let rt = compio::runtime::Runtime::new().unwrap();

    let (sum, panicked) = rt.block_on(async {
        // 外层 compio 上下文内再 block_on 聚合 future（块内执行 spawn 等操作）
        let sum = <SendRt as TrBlockOn<_>>::block_on(concurrent_sum(7));
        let panicked = <SendRt as TrBlockOn<_>>::block_on(panic_propagates());
        (sum, panicked)
    });

    assert_eq!(sum, 42, "7 + 7*2 + 7*3");
    assert!(panicked, "panic 任务必须通过 JoinErr 传播");
    println!("compio cap_spawn_send OK: sum={sum}, join_err_propagates={panicked}");
}
