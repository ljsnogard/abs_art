//! # 设计意图
//!
//! 用**本地投递值**验证「线程本地任务队列」这件事（compio 演示组，与
//! `examples/tokio_demo/cap_spawn_local.rs` 一一对应）。运行时值**自己**就是
//! 本地作用域——投递点与驱动点都在同一个值上。
//!
//! 1. **`!Send` 任务的归宿**：`rt.spawn_local` 投递的任务可以捕获 `Rc` /
//!    `Rc<RefCell<_>>` 这类 `!Send` 数据——这是 `spawn`（抽象层要求 `Send`）
//!    做不到的；
//! 2. **compio 的队列本来就归运行时所有**：compio 的运行时是线程本地的
//!    （`!Send`），`spawn` 与 `spawn_local` 在本后端是**同一个入口**、同一条
//!    队列，差别只在 trait 声明的约束上。因此这里没有 tokio 那样的 `LocalSet`，
//!    也不需要额外的本地队列对象；
//! 3. **驱动点的差异**：tokio 必须经 `run_until` 显式驱动 `LocalSet`；compio
//!    的 `run_until` 就是 `future` 本身（队列由运行时驱动）——但两者**写法相同**，
//!    业务代码 `value.run_until(share_rc(&value))` 一字不改；
//! 4. 多个本地任务共享同一个 `Rc`，在单线程内轮流修改，无需 `Arc` / `Mutex`。
//!
//! # 值语义（v0.4）：本地作用域并回运行时值
//!
//! v0.3 的本地作用域是一个**独立的值**（`LocalScope`，在 compio 上是零大小的），
//! 它可以脱离运行时值单独存在、单独传递，于是「这次 `spawn_local` 投到哪条队列」
//! 与「这次 `delay` 用哪个计时器」是两件互不相干的事。v0.4 把它删除、并回
//! 运行时值：`rt.spawn_local(..)` 与 `rt.run_until(..)` 都打在这一个值上。
//! 代价是必须先有运行时值（`LocalRt::current()`，需要上下文）。
//!
//! # 可以做到
//!
//! - 在一个运行时值上投递捕获 `Rc` 的 future，并 `await` 其 `JoinHandle` 取回
//!   结果；
//! - 多个本地任务共享同一个 `Rc<RefCell<_>>`，顺序修改同一份数据；
//! - 业务函数只依赖 `&R where R: TrLocalScope`，不感知具体后端；
//! - 用 `Runtime<{ SPAWN_LOCAL }>` 把「我要用本地投递」**写下来**。
//!
//! # 不能做到
//!
//! - 没写 `SPAWN_LOCAL` 位还想投递本地任务：那个值不实现 `TrLocalScope`
//!   （负向演示见 `abs_art_demo::strict_mode_check::spawn_local_requires_declaration`
//!   ——这是 v0.4 才真正成立的门控）；
//! - 在没有任何 compio 运行时上下文的线程里**构造值**（`current()` 依赖
//!   `Runtime::current()`，无上下文会 panic）——由集成方（本文件的 `main`）
//!   保证上下文；要脱离上下文构造须改用 `Runtime::with_runtime(rt.clone())`；
//! - 把 `!Send` 任务投递到 `spawn`（抽象层要求 `F: Send`）→ 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，「线程本地」前提保证了它永远不跨线程。

use std::{cell::RefCell, rc::Rc};

use bridge_compio::{Runtime, SPAWN_LOCAL, TrLocalScope};

/// 能力声明：本地投递**必须**被写下来（`SPAWN_LOCAL` 位）。
///
/// 类型别名固定了 `CAPS`，因此表达式位置可以直接写 `LocalRt::current()` 而不必
/// 使用 turbofish。
type LocalRt = Runtime<{ SPAWN_LOCAL }>;

/// 业务函数：在这个运行时值上投两个本地任务，共享同一个 `Rc<RefCell<Vec>>`，
/// 轮流追加元素。
///
/// 与 tokio 组逐字相同——值把后端差异吃掉了，业务侧不需要任何类型参数，
/// 也不需要独立的「作用域参数」。
async fn share_rc<R: TrLocalScope>(rt: &R) -> i32 {
    // Rc 是 !Send：只有「线程本地」的队列才能承载这样的任务；
    // 单线程内共享，RefCell 的可变性检查也在单线程内成立，安全且无锁。
    let shared = Rc::new(RefCell::new(vec![1i32, 2, 3]));

    let s1 = shared.clone();
    let h1 = rt.spawn_local(async move {
        s1.borrow_mut().push(4);
    });

    let s2 = shared.clone();
    let h2 = rt.spawn_local(async move {
        s2.borrow_mut().push(5);
    });

    // 等两个本地任务都完成
    h1.await.unwrap();
    h2.await.unwrap();

    shared.borrow().iter().sum::<i32>()
}

fn main() {
    // compio：运行时本身线程本地，队列归它自己所有
    let rt = compio::runtime::Runtime::new().unwrap();

    // 值必须在运行时上下文内构造；驱动点与 tokio 组写法完全一致
    // （compio 下 run_until 即 future 本身）。
    let out = rt.block_on(async {
        let value = LocalRt::current();
        value.run_until(share_rc(&value)).await
    });

    assert_eq!(out, 15, "1+2+3+4+5");
    println!("compio cap_spawn_local OK: shared_rc_sum={out}");
}
