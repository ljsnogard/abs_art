//! # 设计意图：这是一次**反向演示**
//!
//! 本文件与 `examples/tokio_demo/cap_spawn_send.rs` 一一对应，但结论相反：
//! **compio 声明不了 `SPAWN_SEND` 那一位**（一使用即静态失败），所以这一组演示的
//! 不是「可以跨线程投递」，而是「**不能**跨线程投递，以及应当改用什么」。
//!
//! ## 要验证什么
//!
//! 1. compio 上**声明** `SPAWN_SEND` 位是**构造点**的编译错误（E0277）——
//!    `Runtime::<{ CompioFull | SPAWN_SEND }>` 连值都造不出来
//!    （负向用例见
//!    [`abs_art_demo::strict_mode_check`](https://docs.rs/abs_art-demo) 的
//!    `compio_rejects_spawn_send_at_construction`）；
//! 2. 替代路径是**本地作用域投递**：`Runtime::local_scope()` 取得作用域，
//!    `scope.spawn_local(..)` 投递任务，`scope.run_until(..)` 驱动队列；
//! 3. 同一份「投三个任务再聚合」的业务语义可以照常落地（本示例跑出
//!    `7 + 14 + 21 == 42`，与 tokio 组 `concurrent_sum` 数值一致）；
//! 4. 抽象的句柄契约仍然成立：本地句柄同样是 `TrJoinHandle`，panic 一样经
//!    `JoinErr` 传播——**换的是投递路径，不是句柄抽象**。
//!
//! ## 为什么 compio 必须失败在这里
//!
//! `TrSpawnSend` 的语义是「投递到**全局（跨线程）**工作队列」。compio 没有这样
//! 的队列：
//!
//! - `compio::runtime::Runtime` 内部是 `Rc<Executor>` + `Rc<RefCell<Proactor>>`，
//!   本身就是 `!Send`，绑在创建它的线程上；
//! - 它的 `spawn` 投的是**本线程**运行时的执行器队列，不存在可以被多个线程
//!   共享、被其它线程窃取的工作队列。
//!
//! 因此 `abs_art-compio` 不为 `Runtime<CAPS>` 实现 `TrSpawnSend`，而且做得更彻底：
//! `Runtime<CAPS>` 的类型定义要求 `[(); CAPS]: CompioCaps_`（只对不含
//! `SPAWN_SEND` 位的掩码实现），**声明了该位的值一出现就报 E0277**。这不是遗漏，
//! 而是「如实表达后端前提」：把 compio 也写成实现了 `TrSpawnSend`，就会让业务代码
//! 误以为 `rt.spawn(..)` 意味着跨线程并行——那与事实相反。
//!
//! ## 可以做到
//!
//! - 用 `scope.spawn_local(..)` 投递任务、聚合结果；
//! - 用只依赖 `TrJoinHandle` 的泛型工具函数等待句柄（后端无关）；
//! - 任务 panic 时通过 `JoinErr` 拿到 `Err`，而不是炸掉进程；
//! - `!Send` 数据（如 `Rc`）也能进本地队列——本地投递本来就不要求 `Send`。
//!
//! ## 不能做到
//!
//! - `rt.spawn(..)`：compio 上没有这条路；写 `SPAWN_SEND` 位更早一步就被拒
//!   （**构造点** E0277，而不是调用点「没有 `spawn` 方法」）；
//! - **跨线程并行**：这三个任务只能在**本线程**上交错执行，不会分散到多个
//!   worker 核。这是 compio 的调度事实，抽象层不承诺、也没有办法在这一层
//!   伪造；
//! - 把任务「扔给别的线程」：运行时 `!Send`，队列绑线程；需要多线程时要
//!   在各自的线程上各自建运行时；
//! - 用 `SPAWN_SEND` 能力位换到 `spawn`：这一位在 compio 上是**静态失败**的
//!   声明——能力位只表达业务侧「声明要什么」，能不能给由后端决定，而 compio
//!   的答案是在值出现的瞬间就说「不行」。

use core::future::Future;

use bridge_compio::{CompioFull, CompioRuntime as Runtime, TrJoinHandle, TrLocalScope};

/// 能力声明：本示例写 **`CompioFull`**（`59`，**不含** `SPAWN_SEND`），行为上真正
/// 用到的是 `SPAWN_LOCAL`（取作用域）与 `CLOCK`（无）——见 `main`。
///
/// # 为什么这里**不能**把 `SPAWN_SEND` 写进去
///
/// 本文件的对照说明必须靠「另一条路径」表达：把 `Runtime::<{ CompioFull |
/// SPAWN_SEND }>`（数值 `63`，等于 `abs_art::FULL`）写在**注释**里，而不是写进
/// 类型别名——那一行是**构造点静态失败**（E0277），一旦写进别名，本示例就无法
/// 编译，也就跑不出「替代路径可行」这条正向结论。
///
/// 于是「失败原因是后端兑现不了该位」与「失败原因只是没写能力位」被清楚地区分：
/// 前者是编译期硬拒绝（负向用例在 [`abs_art_demo::strict_mode_check`]），后者只是
/// 方法不可用。
type FullRt = Runtime<{ CompioFull }>;

/// 泛型工具函数：等待**任意后端**的 JoinHandle。
///
/// 只依赖抽象 trait `TrJoinHandle<T>`（它的 supertrait 保证 `H` 是一个
/// `Future<Output = Result<T, H::JoinErr>>`），不感知任何后端句柄类型。它与
/// tokio 组同名函数逐字相同——句柄抽象这一层两后端共享，**投递路径**才是差异点。
async fn join_abstract_<H, T>(handle: H) -> Result<T, H::JoinErr>
where
    H: TrJoinHandle<T> + Future<Output = Result<T, H::JoinErr>>,
{
    handle.await
}

/// 替代路径：用**本地作用域**投三个任务并聚合，等价于 tokio 组的
/// `concurrent_sum`。
async fn local_concurrent_sum_<S>(scope: &S, x: i32) -> i32
where
    S: TrLocalScope,
{
    // 这里本来会写 `rt.spawn(..)`；在 compio 上那条路不存在，改成 scope.spawn_local。
    let h1 = scope.spawn_local(async move { x });
    let h2 = scope.spawn_local(async move { x * 2 });
    let h3 = scope.spawn_local(async move { x * 3 });
    let a = join_abstract_(h1).await.unwrap();
    let b = join_abstract_(h2).await.unwrap();
    let c = join_abstract_(h3).await.unwrap();
    a + b + c
}

/// 替代路径下的 panic 传播：本地句柄同样把任务 panic 报成 `Err`。
async fn local_panic_propagates_<S>(scope: &S) -> bool
where
    S: TrLocalScope,
{
    async fn boom_() -> i32 {
        panic!("任务爆炸");
    }
    let h = scope.spawn_local(boom_());
    join_abstract_(h).await.is_err()
}

fn main() {
    let rt = compio::runtime::Runtime::new().unwrap();

    let (sum, panicked) = rt.block_on(async {
        // 运行时值：本后端的完整能力集 `CompioFull`（59，不含 SPAWN_SEND）。
        let value = FullRt::current();

        // 唯一可用的投递入口：由运行时值交出本地作用域。
        let scope = value.local_scope();

        // 注意：下面这行如果取消注释，会**在构造点编译失败**（E0277:
        // compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力；
        // CompioCaps_ is not implemented for `[(); 63]`）。
        // 这正是本文件要钉住的事实：失败比「没有 `spawn` 方法」更早。
        // let _bad = Runtime::<{ CompioFull | SPAWN_SEND }>::current();
        //
        // 而下面这行（只调 `spawn`、掩码本身合法）失败的原因不同：方法不存在
        // （compio 不实现 TrSpawnSend）。
        // let _ = value.spawn(async { 1 });

        let sum = scope.run_until(local_concurrent_sum_(&scope, 7)).await;
        let panicked = scope.run_until(local_panic_propagates_(&scope)).await;
        (sum, panicked)
    });

    assert_eq!(sum, 42, "7 + 7*2 + 7*3（本地作用域投递）");
    assert!(panicked, "panic 任务必须通过 JoinErr 传播");
    println!(
        "compio cap_spawn_send（反向演示）OK: sum={sum}, join_err_propagates={panicked}, \
         跨线程 spawn 不可用"
    );
}
