//! # 设计意图
//!
//! 用**组合能力** `Runtime<{ BLOCK_ON | SPAWN_BLOCKING }>` 验证「阻塞线程池」：
//!
//! 1. **`spawn_blocking` 抽象**：把 CPU 密集 / 真正阻塞（IO、同步计算）的闭包
//!    投递到专用阻塞线程池，避免占用异步 worker 线程——业务代码只依赖
//!    `TrSpawnBlocking`，不感知后端线程池实现；
//! 2. **能力按需组合**：`BLOCK_ON | SPAWN_BLOCKING` 声明「阻塞等待 + 阻塞池」
//!    两种能力，既不缺也不多；
//! 3. 阻塞任务与异步任务**共存**：阻塞池的任务在后台线程跑，异步侧可以
//!    继续推进自己的逻辑。
//!
//! # 可以做到
//!
//! - `spawn_blocking` 一个 `FnOnce() -> T + Send + 'static` 闭包，返回
//!   `JoinHandle`，`await` 取回 `Result<T, JoinErr>`；
//! - 同时投递多个阻塞任务，交给线程池并行执行；
//! - 与 `block_on` 组合：同步风格的代码也能把重活丢给阻塞池。
//!
//! # 不能做到
//!
//! - 闭包捕获 `!Send` 数据（`F: Send` 约束）→ 编译错误；
//! - 不保证阻塞闭包在**哪个线程**执行——不能依赖线程局部状态
//!   （`thread_local!` 等）；
//! - 闭包必须是 `'static`：不能借用调用者的栈上数据（与 `spawn` 相同的
//!   `'static` 约束，`TrSpawnBlocking` 没有像 `TrBlockOn` 那样放松）；
//! - 阻塞任务没有「取消」语义：一旦投递就会跑完（或 panic 后由 `JoinErr`
//!   传播），抽象层不提供中途取消。

use std::sync::atomic::{AtomicUsize, Ordering};

use bridge_tokio::{BLOCK_ON, SPAWN_BLOCKING, Runtime, TrBlockOn, TrSpawnBlocking};

/// 能力声明：`block_on` + `spawn_blocking`。
type BlockingRt = Runtime<{ BLOCK_ON | SPAWN_BLOCKING }>;

/// 模拟 CPU 密集计算：1..n 的平方和（同步闭包，会占用线程直到算完）。
fn heavy_compute(n: usize) -> usize {
    (0..n).map(|i| i * i).sum()
}

/// 业务函数：往阻塞池投两个任务，异步侧同时推进自己的计数。
///
/// 返回值：(阻塞池两个任务的结果之和, 异步侧推进的计数)。
async fn blocking_plus_async() -> (usize, usize) {
    let counter = std::sync::Arc::new(AtomicUsize::new(0));

    // 两个重活交给阻塞线程池（后台线程并行执行）
    let h1 = <BlockingRt as TrSpawnBlocking<_, usize>>::spawn_blocking(move || {
        heavy_compute(10)
    });
    let h2 = <BlockingRt as TrSpawnBlocking<_, usize>>::spawn_blocking(move || {
        heavy_compute(10)
    });

    // 在 await 阻塞任务的同时，异步侧仍然可以继续做事——worker 线程
    // 没有被重活占死（这正是 spawn_blocking 的设计意图）
    for _ in 0..10 {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    let c1 = h1.await.unwrap();
    let c2 = h2.await.unwrap();
    (c1 + c2, counter.load(Ordering::Relaxed))
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .build()
        .unwrap();

    let (sum, count) = rt.block_on(async {
        // 外层 tokio 上下文内 block_on 聚合 future（块内完成 spawn_blocking）
        <BlockingRt as TrBlockOn<_>>::block_on(blocking_plus_async())
    });

    assert_eq!(sum, 570, "2 * sum(0..10, i^2) = 2 * 285");
    assert_eq!(count, 10, "异步侧计数不受阻塞池影响");
    println!("cap_spawn_blocking OK: blocking_sum={sum}, async_count={count}");
}
