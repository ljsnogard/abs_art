//! # 设计意图
//!
//! 用**本地投递的两件套**验证「线程本地任务队列」这件事：运行时值负责交出
//! **作用域**，作用域负责投递与驱动 `!Send` 任务。
//!
//! 本地投递拆成**声明**与**事实**两半：能力位 `SPAWN_LOCAL` 只负责声明（想用
//! 就得写下来，于是这次升级进入 diff 与 review），而真正能不能投递由**值**
//! 决定。原因：本地投递除了「运行时支持」之外还有一条**环境前提**——必须存在
//! 一个本地队列，并且有人在驱动它。纯类型参数表达不了这条前提（类型对了、
//! 调用点错了，要到运行期才暴露），因此调用点由值把关：
//!
//! 1. **`!Send` 任务的归宿**：`scope.spawn_local` 把任务投递到**线程本地**队列，
//!    因此任务可以捕获 `Rc` / `Rc<RefCell<_>>` 这类 `!Send` 数据——这是
//!    `spawn`（要求 `Send`）做不到的；
//! 2. **前提变成值**：tokio 的本地队列由作用域值持有的 `LocalSet` 承载，
//!    而作用域**只能**经 `Runtime::local_scope()` 取得（该方法要求 `CAPS` 含
//!    `SPAWN_LOCAL`）——拿不到作用域就没有 `spawn_local` 可调；
//! 3. **投递点与驱动点是同一个作用域**：[`TrLocalScope::run_until`]（异步）与
//!    [`TrLocalScope::block_on`]（阻塞）都驱动**这个作用域**的队列，本地任务
//!    才有机会推进；
//! 4. 多个本地任务共享同一个 `Rc`，在单线程内轮流修改，无需 `Arc` / `Mutex`。
//!
//! # 为什么作用域是独立的值（而不是并进运行时值）
//!
//! 本地队列绑定**线程**（tokio 的 `LocalSet` 是 `!Send`），与「运行时」是两件
//! 事：运行时把手可以跨线程共享、可以长期活着，而队列必须由持有者在**创建它的
//! 线程**上驱动。因此最新的抽象把两者分开，并给出一条必须记住的分工——
//! **`Runtime::block_on(f)` 不驱动本地队列，`scope.block_on(f)` 才驱动**。
//! 本示例的两个业务函数分别展示这两者：A 用 `run_until`（异步驱动），
//! B 用 `block_on`（阻塞驱动）。
//!
//! # 可以做到
//!
//! - 在一个作用域上投递捕获 `Rc` 的 future，并 `await` 其 `JoinHandle` 取回结果；
//! - 多个本地任务共享同一个 `Rc<RefCell<_>>`，顺序修改同一份数据；
//! - 业务函数只依赖 `&S where S: TrLocalScope`，不感知具体后端，也不需要运行时
//!   值的类型参数；
//! - 用 `Runtime<{ SPAWN_LOCAL }>` 把「我要用本地投递」**写下来**。
//!
//! # 不能做到
//!
//! - 没写 `SPAWN_LOCAL` 位还想投递本地任务：`local_scope()` 根本不存在，拿不到
//!   作用域（负向演示见
//!   `abs_art_demo::strict_mode_check::local_scope_requires_declaration`
//!   与 `spawn_local_not_on_runtime_value`）；
//! - 把 `spawn_local` 写在**运行时值**上：本地投递的宿主是作用域，运行时值上
//!   没有这个方法；
//! - 不驱动作用域就指望任务推进：tokio 的 `LocalSet` 必须由 `run_until` 或
//!   `block_on` 驱动；若改用**运行时值**的 `block_on`，队列不会被驱动；
//! - 把 `!Send` 任务投递到全局队列（`spawn` 要求 `F: Send`）→ 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，「线程本地」前提保证了它永远不跨线程；
//! - 本示例的 `CAPS` 里没有 `SPAWN_SEND`，因此这里连 `spawn` 也调不了。

use std::{cell::RefCell, rc::Rc};

use bridge_tokio::{TokioRuntime as Runtime, SPAWN_LOCAL, TrLocalScope};

/// 能力声明：本地投递**必须**被写下来（`SPAWN_LOCAL` 位）。
///
/// 类型别名固定了 `CAPS`，因此表达式位置可以直接写 `LocalRt::current()` 而不必
/// 使用 turbofish。
type LocalRt = Runtime<{ SPAWN_LOCAL }>;

/// 业务函数 A：在这个作用域上投两个本地任务，共享同一个 `Rc<RefCell<Vec>>`，
/// 轮流追加元素；由调用方用 `run_until` 驱动队列。
///
/// 注意本函数只依赖 `S: TrLocalScope`——业务侧不需要写任何具体后端类型，
/// 也不需要运行时值：**投递只与队列有关**。
async fn share_rc_<S>(scope: &S) -> i32
where
    S: TrLocalScope,
{
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

/// 业务函数 B：**同步**驱动入口——`scope.block_on` 在等待期间持续驱动队列。
///
/// 对照点：若把 `scope.block_on` 换成运行时值的 `rt.block_on`，本函数体内
/// `spawn_local` 的任务不会被推进，`h1.await` 会永久挂起。
fn share_rc_blocking_<S>(scope: &S) -> i32
where
    S: TrLocalScope,
{
    scope.block_on(async {
        let shared = Rc::new(RefCell::new(vec![10i32, 20]));

        let s1 = shared.clone();
        let h1 = scope.spawn_local(async move {
            s1.borrow_mut().push(30);
        });
        h1.await.unwrap();

        shared.borrow().iter().sum::<i32>()
    })
}

fn main() {
    // ---- A 部分：异步驱动（current_thread 运行时 + scope.run_until）----
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    let out = rt.block_on(async {
        // 两件套：运行时值提供上下文（`current()` 走 `Handle::current()`），
        // 由它交出作用域；驱动点是 scope.run_until。
        let value = LocalRt::current();
        let scope = value.local_scope();
        scope.run_until(share_rc_(&scope)).await
    });

    assert_eq!(out, 15, "1+2+3+4+5");

    // ---- B 部分：阻塞驱动（多线程运行时 + scope.block_on）----
    // tokio 的 TrLocalScope::block_on 基于 block_in_place，因此必须多线程。
    let rt2 = tokio::runtime::Builder::new_multi_thread()
        .build()
        .unwrap();

    let out2 = rt2.block_on(async {
        let value = LocalRt::current();
        let scope = value.local_scope();
        share_rc_blocking_(&scope)
    });

    assert_eq!(out2, 60, "10+20+30");
    println!("cap_spawn_local OK: shared_rc_sum={out}, blocking_sum={out2}");
}
