//! `local_scope`：本地队列由**运行时值**持有——compio 后端，投递点与驱动点同一个值。
//!
//! # 为什么这里不需要调用方提供任何东西
//!
//! compio 的运行时**本身就是线程本地的**（`compio::runtime::Runtime` 内部全是
//! `Rc`，`!Send`，不能跨线程发送），因此：
//!
//! - `Runtime::spawn` **不要求** `F: Send`，`spawn` 与「本地投递」是同一条队列、
//!   同一套语义；
//! - 这条队列归运行时所有，并由运行时自己在 `block_on` / `wait` 期间驱动。
//!
//! v0.3 因此把本地作用域做成一个**零大小**的独立值 `LocalScope`：调用方不需要
//! 提供任何东西，空类型如实表达了这一点。v0.4 把这个独立值**删除**，能力并回
//! [`Runtime`] ——因为 v0.3 的形状有一个副作用：`LocalScope` 可以脱离运行时值
//! 单独存在、单独传递，于是「这次 `spawn_local` 投到哪条队列」与「这次 `delay`
//! 用哪个计时器」是两件互不相干的事。并回运行时值之后，两者由**同一个值**回答。
//!
//! 代价：本地投递现在需要先有一个运行时值（`Runtime::current()` 或
//! `Runtime::with_runtime`），不能再凭一个 ZST 凭空投递。这是有意的——「此刻真的
//! 有本地队列」本来就属于运行时，不属于任何一个可以被复制来复制去的对象。
//!
//! # 保留的语义：[`TrLocalScope::run_until`] 等价于直接 await
//!
//! compio 的队列由运行时自己驱动，**不存在**「需要调用方额外驱动的队列」这回事
//! （对照 tokio 的 `LocalSet::run_until`）。因此本后端的 `run_until(future)`
//! **原样返回 `future`**：驱动来自外层已经在跑的那个 compio `block_on` / `wait`。
//! 这条语义与 v0.3 的 `LocalScope::run_until` 完全一致，v0.4 只是搬家，没有改
//! 语义。
//!
//! 与能力位 [`SPAWN_LOCAL`](abs_art::SPAWN_LOCAL) 的分工见 [`abs_art::caps`]：
//! 位负责「声明」，本 impl 负责「取得」。

use core::future::Future;

use abs_art::{HasSpawnLocal, TrLocalScope};

use crate::{Runtime, join_handle::JoinHandle};

impl<const CAPS: usize> TrLocalScope for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnLocal,
{
    /// 本后端的本地任务句柄：与全局 `spawn` 共用同一个 [`JoinHandle`]。
    type Handle<T> = JoinHandle<T> where T: 'static;

    /// 把 `future` 投递到**本值**的本地队列（compio 的线程本地工作队列）。
    ///
    /// compio 的 `Runtime::spawn` 不要求 `Send`，因此这里可以承载捕获 `Rc` 之类
    /// `!Send` 数据的 future。
    ///
    /// 队列随本值存活，**不随任务句柄存活**——因此
    /// [`TrJoinHandle::detach`](abs_art::TrJoinHandle::detach) 之后任务仍会被运行时
    /// 持续驱动，直到它自己结束。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        self.rt_.spawn(future).into()
    }

    /// 驱动**本值**的本地队列，直到 `future` 完成。
    ///
    /// compio 的队列由运行时自己驱动，所以「驱动队列直到 `future` 完成」就是
    /// `future` 本身——本方法**原样返回 `future`**（语义与 v0.3 的
    /// `LocalScope::run_until` 完全相同）。返回的 future 仍需放在一个正在跑的
    /// compio 上下文里 await（典型：`rt.block_on(value.run_until(fut))`）。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future,
    {
        future
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端「运行时值承载本地队列」的单元测试。

    use std::{cell::Cell, rc::Rc, time::Duration};

    use abs_art::{SPAWN_LOCAL, TrJoinHandle, TrLocalScope};
    use compio::runtime::Runtime as CompioRuntime;

    use crate::Runtime;

    /// 目的：验证 `run_until` / 本地投递能让 `!Send` 任务跑完，且结果能经句柄取回。
    ///
    /// 实施策略：在 compio 运行时里构造运行时值，投递一个捕获 `Rc<u32>` 的本地
    /// 任务，用 `run_until` 驱动并 await 其句柄。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `run_until` 没有把 future 交给正在跑的
    /// compio 上下文（或本地队列没被驱动），await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            value
                .run_until(async {
                    let rc = Rc::new(6u32);
                    value.spawn_local(async move { *rc * 7 }).await.unwrap()
                })
                .await
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归运行时所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 实施策略：投递一个置位 `Rc<Cell<bool>>` 的本地任务后立即 `detach()`，再用
    /// 同一个运行时值的 `delay` 反复让出，等标志置位（总共最多 1 秒）。
    ///
    /// 通过依据：标志在期限内被置位；若 detach 实际取消了任务（compio 的
    /// `JoinHandle` 在 drop 时会 cancel，必须走原生 `detach`），断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        use abs_art::TrDelay;

        let rt = CompioRuntime::new().unwrap();

        rt.block_on(async {
            let value = crate::current();
            value
                .run_until(async {
                    let flag = Rc::new(Cell::new(false));
                    let task_flag = flag.clone();

                    let handle = value.spawn_local(async move {
                        task_flag.set(true);
                    });
                    handle.detach();

                    let mut elapsed = 0u32;
                    while !flag.get() && elapsed < 1_000 {
                        value.delay(Duration::from_millis(1)).await;
                        elapsed += 1;
                    }
                    assert!(flag.get(), "detach 后本地任务未被推进");
                })
                .await;
        });
    }

    /// 目的：验证「声明能力位」路径——只写 `SPAWN_LOCAL` 的运行时值确实可用本地投递。
    ///
    /// 实施策略：把 CAPS 写成只含 `SPAWN_LOCAL`，在 compio 运行时上下文内构造值
    /// 并跑一个捕获 `Rc` 的 `!Send` 任务。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错（例如标在别的
    /// 位上），本测试将无法编译。
    #[test]
    fn declared_cap_gives_usable_local_queue() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = Runtime::<{ SPAWN_LOCAL }>::current();
            value
                .run_until(async {
                    let rc = Rc::new(6u32);
                    value.spawn_local(async move { *rc * 7 }).await.unwrap()
                })
                .await
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证同一个运行时值的两个克隆共享**同一条**本地队列（`Rc` 句柄簇）。
    ///
    /// 实施策略：克隆值，用克隆体投递任务，用原值 `run_until` 驱动，再 await 句柄。
    ///
    /// 通过依据：取回 5——若两个克隆指向两份运行时，原值驱动的上下文不会推进克隆体
    /// 投递的任务，await 会挂起。
    #[test]
    fn clones_share_one_local_queue() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let clone = value.clone();
            let handle = clone.spawn_local(async { 5u32 });
            value.run_until(handle).await.unwrap()
        });

        assert_eq!(out, 5);
    }

    /// 目的：验证 `TrLocalScope::Handle` 与 `crate::JoinHandle` 是同一个类型。
    ///
    /// 实施策略：把 `spawn_local` 交回的句柄直接传给一个形参类型为
    /// `crate::JoinHandle<u32>` 的函数。
    ///
    /// 通过依据：编译通过即为通过（类型相等）。
    #[test]
    fn handle_type_is_the_shared_join_handle() {
        fn take_handle_(_: crate::JoinHandle<u32>) {}

        let rt = CompioRuntime::new().unwrap();
        rt.block_on(async {
            let value = crate::current();
            take_handle_(value.spawn_local(async { 1u32 }));
        });
    }
}
