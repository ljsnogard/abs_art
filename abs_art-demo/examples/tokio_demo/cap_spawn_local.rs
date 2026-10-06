//! # 设计意图
//!
//! 用**本地投递值**验证「线程本地任务队列」这件事。运行时值**自己**就是本地
//! 作用域——投递点与驱动点都在同一个值上。
//!
//! 本地投递拆成**声明**与**事实**两半：能力位 `SPAWN_LOCAL` 只负责声明（想用
//! 就得写下来，于是这次升级进入 diff 与 review），而真正能不能投递由**值**
//! 决定。原因：本地投递除了「运行时支持」之外还有一条**环境前提**——必须存在
//! 一个本地队列，并且有人在驱动它。纯类型参数表达不了这条前提（类型对了、
//! 调用点错了，要到运行期才暴露），因此调用点由值把关：
//!
//! 1. **`!Send` 任务的归宿**：`rt.spawn_local` 把任务投递到**线程本地**队列，
//!    因此任务可以捕获 `Rc` / `Rc<RefCell<_>>` 这类 `!Send` 数据——这是
//!    `spawn`（要求 `Send`）做不到的；
//! 2. **前提变成值**：tokio 的本地队列归这个运行时值持有的 `LocalSet` 所有，
//!    **拿不到实现 `TrLocalScope` 的值就没有 `spawn_local` 可调**，这比
//!    「没声明能力位就编译不过」是更强的保证；
//! 3. **投递点与驱动点是同一个值**：[`run_until`](TrLocalScope::run_until)
//!    驱动这个值的队列，本地任务才有机会推进；
//! 4. 多个本地任务共享同一个 `Rc`，在单线程内轮流修改，无需 `Arc` / `Mutex`。
//!
//! # 值语义（v0.4）：本地队列并回运行时值
//!
//! v0.3 的本地作用域是一个**独立的值**（`LocalScope`）：它可以脱离运行时类型
//! 单独存在、单独传递、单独驱动，于是「这次 `spawn_local` 投到哪条队列」与
//! 「这次 `delay` 用的是谁的计时器」是两个互不相干的答案。v0.4 把队列并回
//! 运行时值：`rt.spawn_local(..)` 与 `rt.run_until(..)` 都打在这一个值上。
//! 代价是运行时值必须由处于上下文的一方构造（`LocalRt::current()`）。
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
//! - 不驱动本地队列就指望任务推进：tokio 的队列由本值持有的 `LocalSet` 驱动，
//!   必须经 `run_until`（或 `block_on`）提供驱动点；
//! - 把 `!Send` 任务投递到全局队列（`spawn` 要求 `F: Send`）→ 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，「线程本地」前提保证了它永远不跨线程。

use std::{cell::RefCell, rc::Rc};

use bridge_tokio::{Runtime, SPAWN_LOCAL, TrLocalScope};

/// 能力声明：本地投递**必须**被写下来（`SPAWN_LOCAL` 位）。
///
/// 类型别名固定了 `CAPS`，因此表达式位置可以直接写 `LocalRt::current()` 而不必
/// 使用 turbofish。
type LocalRt = Runtime<{ SPAWN_LOCAL }>;

/// 业务函数：在这个运行时值上投两个本地任务，共享同一个 `Rc<RefCell<Vec>>`，
/// 轮流追加元素。
///
/// 注意本函数只依赖 `R: TrLocalScope`——业务侧不需要写任何具体后端类型，
/// 也不需要独立的「作用域参数」：运行时值本身就是作用域。
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

    // 等两个本地任务都完成（值在驱动队列，句柄因此能拿到结果）
    h1.await.unwrap();
    h2.await.unwrap();

    shared.borrow().iter().sum::<i32>()
}

fn main() {
    // tokio 需要一个运行时来驱动；本地队列由抽象层的运行时值持有
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    // 值必须在运行时上下文内构造：`current()` 走 `Handle::current()`。
    // 驱动点是 value.run_until —— 在等待业务 future 期间持续推进本地队列。
    let out = rt.block_on(async {
        let value = LocalRt::current();
        value.run_until(share_rc(&value)).await
    });

    assert_eq!(out, 15, "1+2+3+4+5");
    println!("cap_spawn_local OK: shared_rc_sum={out}");
}
