//! # 设计意图
//!
//! 用**本地投递的两件套**验证「线程本地任务队列」这件事：运行时值负责交出
//! **本线程队列的别名**，作用域负责投递与驱动 `!Send` 任务。
//!
//! 本地投递拆成**声明**与**事实**两半：能力位 `SPAWN_LOCAL` 只负责声明（想用
//! 就得写下来，于是这次升级进入 diff 与 review），而真正能不能投递由**值**
//! 决定。原因：本地投递除了「运行时支持」之外还有一条**环境前提**——必须有一条
//! 本线程的队列，并且有人在驱动它。纯类型参数表达不了这条前提（类型对了、
//! 调用点错了，要到运行期才暴露），因此调用点由值把关：
//!
//! 1. **`!Send` 任务的归宿**：`scope.spawn_local` 把任务投递到**线程本地**队列，
//!    因此任务可以捕获 `Rc` / `Rc<RefCell<_>>` 这类 `!Send` 数据——这是
//!    `spawn`（要求 `Send`）做不到的；
//! 2. **前提变成值**：tokio 的本地队列在**本线程的 `thread_local!`** 里，作用域
//!    只是那条队列的别名，而作用域**只能**经 `Runtime::local_scope()` 取得
//!    （该方法要求 `CAPS` 含 `SPAWN_LOCAL`）——拿不到作用域就没有 `spawn_local`
//!    可调；同一线程上多次取得拿到的是**同一条**队列（`Clone` 同理）；
//! 3. **投递点与驱动点是同一个作用域**：[`TrLocalScope::run_until`] 驱动本线程的
//!    队列，本地任务才有机会推进；
//! 4. 多个本地任务共享同一个 `Rc`，在单线程内轮流修改，无需 `Arc` / `Mutex`。
//!
//! # 阻塞与驱动是两件事
//!
//! 抽象层的作用域上**没有**阻塞入口（tokio 的 `block_in_place` 在 `LocalSet` 内
//! 被 tokio 自己禁止，三个后端对「作用域阻塞」给不出同一个承诺），所以
//! 「同步地驱动队列」要**组合**两件事：
//!
//! ```text
//! rt.block_on(scope.run_until(f))   // 阻塞在运行时值、驱动在作用域
//! ```
//!
//! 只写 `rt.block_on(f)` 不驱动队列；只写 `scope.run_until(f)` 需要一个外部的
//! 驱动源。本示例的两个业务函数分别展示异步驱动（A）与这条组合（B）。
//!
//! # 可以做到
//!
//! - 在一个作用域上投递捕获 `Rc` 的 future，并 `await` 其 `JoinHandle` 取回结果；
//! - 多个本地任务共享同一个 `Rc<RefCell<_>>`，顺序修改同一份数据；
//! - 业务函数 A 只依赖 `&S where S: TrLocalScope`，不感知具体后端；
//! - 用 `Runtime<{ SPAWN_LOCAL }>` 把「我要用本地投递」**写下来**；
//! - 需要同步等待时按需再声明 `BLOCK_ON`，用上面的组合写法。
//!
//! # 不能做到
//!
//! - 没写 `SPAWN_LOCAL` 位还想投递本地任务：`local_scope()` 根本不存在，拿不到
//!   作用域（负向演示见
//!   `abs_art_demo::strict_mode_check::local_scope_requires_declaration`
//!   与 `spawn_local_not_on_runtime_value`）；
//! - 把 `spawn_local` 写在**运行时值**上：本地投递的宿主是作用域，运行时值上
//!   没有这个方法；
//! - 不驱动作用域就指望任务推进：本线程的 `LocalSet` 必须由 `run_until` 驱动；
//!   若改用**运行时值**的 `block_on` 而不套 `run_until`，队列不会被驱动；
//! - 在作用域上找阻塞入口：`scope.block_on(..)` 已不是抽象层的能力；
//! - 把 `!Send` 任务投递到全局队列（`spawn` 要求 `F: Send`）→ 编译错误；
//! - `Rc` 跨线程共享：`Rc` 本身 `!Send`，「线程本地」前提保证了它永远不跨线程；
//! - 本示例的 `CAPS` 里没有 `SPAWN_SEND`，因此这里连 `spawn` 也调不了。

use std::{cell::RefCell, rc::Rc};

use bridge_tokio::{BLOCK_ON, SPAWN_LOCAL, TokioRuntime as Runtime, TrBlockOn, TrLocalScope};

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

/// 业务函数 B：**同步**入口——阻塞在运行时值、驱动在作用域，两者组合。
///
/// 对照点：把 `scope.run_until` 去掉（只 `rt.block_on(..)`），本函数体内
/// `spawn_local` 的任务不会被推进，`h1.await` 会永久挂起。
fn share_rc_blocking_<R, S>(rt: &R, scope: &S) -> i32
where
    R: TrBlockOn,
    S: TrLocalScope,
{
    rt.block_on(scope.run_until(async {
        let shared = Rc::new(RefCell::new(vec![10i32, 20]));

        let s1 = shared.clone();
        let h1 = scope.spawn_local(async move {
            s1.borrow_mut().push(30);
        });
        h1.await.unwrap();

        shared.borrow().iter().sum::<i32>()
    }))
}

fn main() {
    // ---- A 部分：异步驱动（current_thread 运行时 + scope.run_until）----
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    let out = rt.block_on(async {
        // 两件套：运行时值提供上下文（`current()` 走 `Handle::current()`），
        // 由它交出本线程队列的别名；驱动点是 scope.run_until。
        let value = LocalRt::current();
        let scope = value.local_scope();
        scope.run_until(share_rc_(&scope)).await
    });

    assert_eq!(out, 15, "1+2+3+4+5");

    // ---- B 部分：阻塞驱动（值阻塞 + 作用域驱动队列）----
    // 这一部分多要一位 `BLOCK_ON`——阻塞发生在运行时值上，就得把它写下来。
    // 并用 `with_handle` 在运行时上下文**之外**造值：tokio 的 `block_on` 落到
    // 句柄上，不需要先进上下文（若已在多线程上下文内，则走 `block_in_place`）。
    let rt2 = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    let value2 = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::with_handle(rt2.handle().clone());
    let scope2 = value2.local_scope();
    let out2 = share_rc_blocking_(&value2, &scope2);

    assert_eq!(out2, 60, "10+20+30");
    println!("cap_spawn_local OK: shared_rc_sum={out}, blocking_sum={out2}");
}
