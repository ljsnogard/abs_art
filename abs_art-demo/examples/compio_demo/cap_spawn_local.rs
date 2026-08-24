//! # 设计意图
//!
//! 用**单能力** `Runtime<{ SPAWN_LOCAL }>` 验证「线程本地任务队列」这一能力
//! （compio 演示组，与 `examples/tokio_demo/cap_spawn_local.rs` 一一对应）：
//!
//! 1. **`!Send` 任务的归宿**：`spawn_local` 投递的任务可以捕获 `Rc` /
//!    `Rc<RefCell<_>>` 这类 `!Send` 数据——这是 `spawn`（要求 `Send`）
//!    做不到的；
//! 2. **compio 的差异**：compio 运行时**本身就是线程本地**的（`!Send`，
//!    不能跨线程发送），因此它的 `spawn_local` 与 `spawn` 行为一致，**没有
//!    tokio 的 `LocalSet` 概念**——`Rc` 任务直接在 `rt.block_on` 上下文里
//!    就能跑，不需要额外包装；「不能做到什么」因此也少了一条（tokio 组的
//!    「必须在 LocalSet 内」在 compio 下不存在，改由「必须在运行时上下文内」
//!    承担）；
//! 3. 多个本地任务共享同一个 `Rc`，在单线程内轮流修改，无需 `Arc` / `Mutex`。
//!
//! # 可以做到
//!
//! - `spawn_local` 一个捕获 `Rc` 的 future，并 `await` 其 `JoinHandle` 取回结果；
//! - 多个本地任务共享同一个 `Rc<RefCell<_>>`，顺序修改同一份数据；
//! - 能力声明精确到「只要本地投递」。
//!
//! # 不能做到
//!
//! - 在没有任何 compio 运行时上下文的线程里调用 `spawn_local`（实现依赖
//!   `Runtime::with_current`，无上下文会 panic）——由集成方（本文件的
//!   `main`）保证上下文；
//! - 把 `!Send` 任务投递到 `spawn`（需要 `SPAWN_SEND` 位且要求 `F: Send`）
//!   → 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，「线程本地」前提保证了它永远不
//!   跨线程——想跨线程共享请改用 `Arc` + `spawn`（那是 `SPAWN_SEND` 能力的
//!   领域）。

use std::{cell::RefCell, rc::Rc};

use bridge_compio::{SPAWN_LOCAL, Runtime, TrSpawnLocal};

/// 能力声明：只请求 `spawn_local` 一种能力（最小权限）。
type LocalRt = Runtime<{ SPAWN_LOCAL }>;

/// 业务函数：两个本地任务共享同一个 `Rc<RefCell<Vec>>`，轮流追加元素。
///
/// 与 tokio 组逐字相同——区别只在 main 里：compio 不需要 `LocalSet`，
/// `rt.block_on` 本身就是线程本地上下文。
async fn share_rc() -> i32 {
    // Rc 是 !Send：只有「线程本地」的 spawn_local 才能承载这样的任务；
    // 单线程内共享，RefCell 的可变性检查也在单线程内成立，安全且无锁。
    let shared = Rc::new(RefCell::new(vec![1i32, 2, 3]));

    let s1 = shared.clone();
    let h1 = <LocalRt as TrSpawnLocal<_>>::spawn_local(async move {
        s1.borrow_mut().push(4);
    });

    let s2 = shared.clone();
    let h2 = <LocalRt as TrSpawnLocal<_>>::spawn_local(async move {
        s2.borrow_mut().push(5);
    });

    // 等两个本地任务都完成（JoinHandle 的 await 在这里直接可用）
    h1.await.unwrap();
    h2.await.unwrap();

    shared.borrow().iter().sum::<i32>()
}

fn main() {
    // compio：没有 LocalSet，运行时本身线程本地，rt.block_on 即上下文
    let rt = compio::runtime::Runtime::new().unwrap();

    let out = rt.block_on(async { share_rc().await });

    assert_eq!(out, 15, "1+2+3+4+5");
    println!("compio cap_spawn_local OK: shared_rc_sum={out}");
}
