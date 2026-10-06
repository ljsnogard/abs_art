//! # 设计意图
//!
//! 用**本地投递的两件套**验证「线程本地任务队列」这件事（compio 演示组，与
//! `examples/tokio_demo/cap_spawn_local.rs` 一一对应）：
//!
//! 1. **`!Send` 任务的归宿**：`scope.spawn_local` 投递的任务可以捕获 `Rc` /
//!    `Rc<RefCell<_>>` 这类 `!Send` 数据——这是全局限定 `F: Send` 的
//!    `spawn` 做不到的；
//! 2. **compio 的队列本来就归运行时所有**：compio 的运行时是线程本地的，
//!    它的执行器不可分离，因此 compio 侧的作用域就是**那份运行时的别名**——
//!    队列由运行时持有并由运行时自己驱动，`run_until(future)` 在语义上等价于直接
//!    `await future`。但**写法与 tokio 组逐字相同**：
//!    `rt.local_scope()` + `scope.spawn_local(..)` + `scope.run_until(..)`；
//! 3. **取得入口被能力位门控**：`local_scope()` 要求 `CAPS` 含 `SPAWN_LOCAL`，
//!    所以「开始用本地投递」这件事必然先写在类型别名里；
//! 4. 多个本地任务共享同一个 `Rc`，在单线程内轮流修改，无需 `Arc` / `Mutex`。
//!
//! # 两件套的分工在 compio 上一样成立
//!
//! 计时/时刻在**运行时值**上（见 `cap_delay`），`spawn_local` / `run_until` 在
//! **作用域**上；作用域上**没有**阻塞入口——抽象层把它删掉了，因为 tokio 的
//! `block_in_place` 在 `LocalSet` 内被 tokio 自己禁止，三个后端对「作用域阻塞」
//! 给不出同一个承诺。需要同步等待时把两者**组合**起来：
//! `rt.block_on(scope.run_until(..))`（阻塞在运行时值、驱动在作用域）。
//!
//! # 可以做到
//!
//! - 在作用域上投递捕获 `Rc` 的 future，并 `await` 其 `JoinHandle` 取回结果；
//! - 多个本地任务共享同一个 `Rc<RefCell<_>>`，顺序修改同一份数据；
//! - 用 `rt.block_on(scope.run_until(..))` **同步**驱动队列——compio 的值自带
//!   上下文，因此这条组合在上下文之外也能用；
//! - 业务函数只依赖 `&S where S: TrLocalScope`，不感知具体后端。
//!
//! # 不能做到
//!
//! - 没写 `SPAWN_LOCAL` 位还想投递本地任务：`local_scope()` 根本不存在
//!   （负向演示见
//!   `abs_art_demo::strict_mode_check::local_scope_requires_declaration`）；
//! - 把 `spawn_local` 写在**运行时值**上：本地投递的宿主是作用域
//!   （`spawn_local_not_on_runtime_value`）；
//! - 在作用域上找阻塞入口：`scope.block_on(..)` 已不是抽象层的能力；
//! - 把 `!Send` 任务「跨线程」投递：compio 的运行时与队列都绑线程，
//!   这也是它不实现 `TrSpawnSend` 的原因（见 `cap_spawn_send.rs`）；
//! - 依赖作用域提供计时：作用域上**没有** `delay` / `now`，那些在运行时值上。

use std::{cell::RefCell, rc::Rc};

use bridge_compio::{BLOCK_ON, CompioRuntime as Runtime, SPAWN_LOCAL, TrBlockOn, TrLocalScope};

/// 能力声明：本地投递**必须**被写下来（`SPAWN_LOCAL` 位）。
///
/// 类型别名固定了 `CAPS`，因此表达式位置可以直接写 `LocalRt::current()` 而不必
/// 使用 turbofish。
type LocalRt = Runtime<{ SPAWN_LOCAL }>;

/// 业务函数 A：在这个作用域上投两个本地任务，共享同一个 `Rc<RefCell<Vec>>`，
/// 轮流追加元素；由调用方用 `run_until` 驱动队列。
///
/// 与 tokio 组逐字相同——两件套把后端差异吃掉了，业务侧不需要任何类型参数。
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

    // 等两个本地任务都完成
    h1.await.unwrap();
    h2.await.unwrap();

    shared.borrow().iter().sum::<i32>()
}

/// 业务函数 B：**同步**入口——阻塞在运行时值、驱动在作用域，两者组合。
///
/// 对照点：只写 `rt.block_on(..)` 而**不**套 `run_until`，本函数体内
/// `spawn_local` 的任务不会被推进，`h1.await` 会永久挂起。
///
/// compio 的队列由运行时自己驱动，所以这里的 `run_until` 只是「把 future 交回
/// 外层驱动」，没有 tokio 那样的多线程前提。
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
    // ---- A 部分：异步驱动（compio 上下文 + scope.run_until）----
    // compio：运行时本身线程本地，队列归它自己所有。
    let rt = compio::runtime::Runtime::new().unwrap();

    let out = rt.block_on(async {
        let value = LocalRt::current();
        let scope = value.local_scope();
        scope.run_until(share_rc_(&scope)).await
    });

    assert_eq!(out, 15, "1+2+3+4+5");

    // ---- B 部分：阻塞驱动（值阻塞 + 作用域驱动队列）----
    // compio 的值自带上下文：用 `with_runtime` 在上下文之外造值也能驱动。
    // 这一部分多要一位 `BLOCK_ON`——阻塞发生在运行时值上，就得把它写下来。
    let rt2 = compio::runtime::Runtime::new().unwrap();
    let value2 = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::with_runtime(rt2.clone());
    let scope2 = value2.local_scope();
    let out2 = share_rc_blocking_(&value2, &scope2);

    assert_eq!(out2, 60, "10+20+30");
    println!("compio cap_spawn_local OK: shared_rc_sum={out}, blocking_sum={out2}");
}
