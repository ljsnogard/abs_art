//! # 设计意图
//!
//! 用**本地作用域值** [`LocalScope`] 验证「线程本地任务队列」这件事
//! （compio 演示组，与 `examples/tokio_demo/cap_spawn_local.rs` 一一对应）。
//!
//! 1. **`!Send` 任务的归宿**：作用域投递的任务可以捕获 `Rc` / `Rc<RefCell<_>>`
//!    这类 `!Send` 数据——这是 `spawn`（要求 `Send`）做不到的；
//! 2. **同名但内容不同**：tokio 组的 [`LocalScope`] 装着一个 `LocalSet`，
//!    而 compio 组的它是**零大小**的——compio 的运行时本身就是线程本地的
//!    （`Runtime` 是 `!Send`），队列归运行时所有并由它在 `block_on` 期间驱动，
//!    调用方**无需提供任何东西**。空类型不是占位符，而是如实表达这一点；
//! 3. **驱动点的差异**：tokio 必须经 `run_until` 显式驱动 `LocalSet`；
//!    compio 的 `run_until` 就是 `future` 本身（队列由运行时驱动）——但两者
//!    **写法相同**，业务代码 `OUTER_DRIVER(scope.run_until(fut))` 一字不改；
//! 4. 多个本地任务共享同一个 `Rc`，在单线程内轮流修改，无需 `Arc` / `Mutex`。
//!
//! # 可以做到
//!
//! - 经作用域投递一个捕获 `Rc` 的 future，并 `await` 其 `JoinHandle` 取回结果；
//! - 多个本地任务共享同一个 `Rc<RefCell<_>>`，顺序修改同一份数据；
//! - 业务函数只依赖 `&impl TrLocalScope`，不感知具体后端；
//! - 经 `Runtime<{ SPAWN_LOCAL }>::local_scope()` 声明式地取得作用域。
//!
//! # 不能做到
//!
//! - 不经作用域投递本地任务：`Runtime<CAPS>` 上已经没有 `spawn_local`
//!   （负向演示见 `abs_art_demo::strict_mode_check::no_spawn_local_on_runtime_type`）；
//! - 在没有任何 compio 运行时上下文的线程里投递（实现依赖 `Runtime::with_current`，
//!   无上下文会 panic）——由集成方（本文件的 `main`）保证上下文；
//! - 把 `!Send` 任务投递到 `spawn`（要求 `F: Send`）→ 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，「线程本地」前提保证了它永远不跨线程。

use std::{cell::RefCell, rc::Rc};

use bridge_compio::{LocalScope, Runtime, SPAWN_LOCAL, TrLocalScope};

/// 业务函数：两个本地任务共享同一个 `Rc<RefCell<Vec>>`，轮流追加元素。
///
/// 与 tokio 组逐字相同——作用域把后端差异吃掉了，业务侧不需要任何类型参数。
async fn share_rc(scope: &LocalScope) -> i32 {
    // Rc 是 !Send：只有「线程本地」的队列才能承载这样的任务；
    // 单线程内共享，RefCell 的可变性检查也在单线程内成立，安全且无锁。
    let shared = Rc::new(RefCell::new(vec![1i32, 2, 3]));

    let s1 = shared.clone();
    let h1 = scope.spawn_local(async move {
        s1.borrow_mut().push(4);
    });

    let s2 = shared.clone();
    let h2 = scope.spawn_local(async move {
        s2.borrow_mut().push(5);
    });

    // 等两个本地任务都完成
    h1.await.unwrap();
    h2.await.unwrap();

    shared.borrow().iter().sum::<i32>()
}

fn main() {
    // 声明「我要用本地投递」：`SPAWN_LOCAL` 位因此必须被写下来
    type LocalRt = Runtime<{ SPAWN_LOCAL }>;
    let scope = LocalRt::local_scope();

    // compio：运行时本身线程本地，队列归它自己所有；作用域是空的
    let rt = compio::runtime::Runtime::new().unwrap();

    // 与 tokio 组写法完全一致的驱动点（compio 下 run_until 即 future 本身）
    let out = rt.block_on(scope.run_until(share_rc(&scope)));

    assert_eq!(out, 15, "1+2+3+4+5");
    println!("compio cap_spawn_local OK: shared_rc_sum={out}");
}
