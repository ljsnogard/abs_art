//! # 设计意图
//!
//! 用**本地作用域值** [`LocalScope`] 验证「线程本地任务队列」这件事。
//!
//! 自 v0.3 起，本地投递拆成**声明**与**事实**两半：能力位 `SPAWN_LOCAL` 只负责
//! 声明（想用就得写下来，于是这次升级进入 diff 与 review），而真正能不能投递由
//! **值**决定。原因：本地投递除了「运行时支持」之外还有一条**环境前提**——
//! 必须存在一个本地队列，并且有人在驱动它。纯类型参数表达不了这条前提（类型对了、
//! 调用点错了，要到运行期才暴露），因此调用点改由值把关：
//!
//! 1. **`!Send` 任务的归宿**：`LocalScope::spawn_local` 把任务投递到**线程本地**
//!    队列，因此任务可以捕获 `Rc` / `Rc<RefCell<_>>` 这类 `!Send` 数据——这是
//!    `spawn`（要求 `Send`）做不到的；
//! 2. **前提变成值**：tokio 的本地队列归调用方的 `LocalSet` 所有，[`LocalScope`]
//!    就是这个 `LocalSet` 的持有者——**拿不到作用域值就没有 `spawn_local` 可调**，
//!    这比「没声明能力位就编译不过」是更强的保证；
//! 3. **投递点与驱动点是同一个值**：[`run_until`](TrLocalScope::run_until) /
//!    [`block_on`](TrLocalScope::block_on) 负责驱动队列，本地任务才有机会推进；
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
//! - 不驱动作用域就指望任务推进：tokio 的队列由 `LocalSet` 驱动，必须经
//!   `run_until` / `block_on` 提供驱动点（负向演示见 `abs_art-smoke` 的 B 用例）；
//! - 把 `!Send` 任务投递到全局队列（`spawn` 要求 `F: Send`）→ 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，「线程本地」前提保证了它永远不跨线程。

use std::{cell::RefCell, rc::Rc};

use bridge_tokio::{LocalScope, Runtime, SPAWN_LOCAL, TrLocalScope};

/// 业务函数：两个本地任务共享同一个 `Rc<RefCell<Vec>>`，轮流追加元素。
///
/// 注意本函数只依赖 `&LocalScope`——业务侧不需要写任何运行时类型参数。
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

    // 等两个本地任务都完成（作用域在驱动队列，句柄因此能拿到结果）
    h1.await.unwrap();
    h2.await.unwrap();

    shared.borrow().iter().sum::<i32>()
}

fn main() {
    // 声明「我要用本地投递」：`SPAWN_LOCAL` 位因此必须被写下来，这次「升级」
    // 于是会出现在类型别名、diff 与 code review 里（见 `strict_mode_check`
    // 的 `local_scope_requires_declaration`）。
    type LocalRt = Runtime<{ SPAWN_LOCAL }>;
    let scope = LocalRt::local_scope();

    // tokio 需要一个运行时来驱动；本地队列本身由 LocalScope 持有
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    // 驱动点：scope.run_until —— 在等待业务 future 期间持续推进本地队列
    let out = rt.block_on(scope.run_until(share_rc(&scope)));

    assert_eq!(out, 15, "1+2+3+4+5");
    println!("cap_spawn_local OK: shared_rc_sum={out}");
}
