//! # 设计意图
//!
//! 用**单能力** `Runtime<{ SPAWN_LOCAL }>` 验证「线程本地任务队列」这一能力：
//!
//! 1. **`!Send` 任务的归宿**：`spawn_local` 把任务投递到**线程本地**队列，
//!    因此任务可以捕获 `Rc` / `Rc<RefCell<_>>` 这类 `!Send` 数据——这是
//!    `spawn`（要求 `Send`）做不到的；
//! 2. **能力可以精确到只声明 `SPAWN_LOCAL`**：本示例的业务代码只需要
//!    「本地投递」，就不声明 `BLOCK_ON` / `SPAWN_SEND`——未声明即编译期拒绝，
//!    能力模型的最小权限原则；
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
//! - 在 `LocalSet` 上下文之外调用 `spawn_local`（tokio 后端）→ **运行期 panic**
//!   （tokio 要求 `spawn_local` 必须在 `LocalSet` 内；这是后端契约，由集成方
//!   保证——本示例的 `main` 用 `LocalSet::block_on` 提供该上下文）；
//! - 把 `!Send` 任务投递到全局队列（`spawn` 需要 `SPAWN_SEND` 位且要求
//!   `F: Send`）→ 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，`spawn_local` 的「线程本地」前提
//!   保证了它永远不跨线程——想跨线程共享请改用 `Arc` + `spawn`（那是
//!   `SPAWN_SEND` 能力的领域）。

use std::{cell::RefCell, rc::Rc};

use bridge_tokio::{SPAWN_LOCAL, Runtime, TrSpawnLocal};

/// 能力声明：只请求 `spawn_local` 一种能力（最小权限）。
type LocalRt = Runtime<{ SPAWN_LOCAL }>;

/// 业务函数：两个本地任务共享同一个 `Rc<RefCell<Vec>>`，轮流追加元素。
///
/// 必须在 `LocalSet` 上下文内调用（tokio 后端），由调用方保证。
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
    // spawn_local 需要「线程本地」上下文：current_thread 运行时 + LocalSet
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    let out = local.block_on(&rt, async { share_rc().await });

    assert_eq!(out, 15, "1+2+3+4+5");
    println!("cap_spawn_local OK: shared_rc_sum={out}");
}
