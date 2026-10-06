//! `local_scope`：本地队列由**运行时值**持有——投递点与驱动点都是这个值。
//!
//! # 为什么 smol 这边必须自建本地队列
//!
//! smol 2.x **没有**内建的 `spawn_local`：它只提供全局执行器（`smol::spawn`，
//! 进程级单例，见 `spawn_send.rs`）。本地队列只能由使用方用
//! `async_executor::LocalExecutor`（smol 重导出为 `smol::LocalExecutor`）自建，
//! 并且**自己驱动**（`LocalExecutor::run` / `tick`）。
//!
//! v0.3 之前的实现把执行器塞进 `JoinHandle`、在句柄被 poll 时顺带 tick 它，于是
//! 「本地任务能否推进」变成了「调用方有没有在 poll 句柄」的函数，`detach()` 消费
//! 句柄还会连带销毁执行器、把任务当场取消。
//!
//! v0.4 起执行器归**运行时值**所有（`Rc<LocalExecutor<'static>>`）：
//!
//! - 投递走 [`Runtime`] 值的 `spawn_local`（`LocalExecutor::spawn`，队列未被驱动时
//!   也能投递、不 panic，正是「先建队列、后驱动」需要的语义）；
//! - 驱动走 [`TrLocalScope::run_until`]（异步）或
//!   [`TrBlockOn::block_on`](abs_art::TrBlockOn::block_on)（阻塞，见 `block_on.rs`）；
//! - 克隆运行时值即共享**同一条**队列（`Rc` 共享），句柄只持有 `smol::Task`，
//!   因此 `detach()` 之后任务仍会被持续驱动。
//!
//! 与 tokio 的形状差异只在于「值持有的是什么」：tokio 持 `Rc<LocalSet>`（还要先
//! 有一条环境运行时来给它投递），smol 持 `Rc<LocalExecutor>`（自足，无先决条件）。

use core::future::Future;

use abs_art::{HasSpawnLocal, TrLocalScope};

use crate::{Runtime, join_handle::JoinHandle};

impl<const CAPS: usize> TrLocalScope for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnLocal,
{
    /// 本后端的本地任务句柄：与全局 `spawn` / `spawn_blocking` 共用同一个
    /// [`JoinHandle`]（它只是 `smol::Task` 的薄包装，不持有队列）。
    type Handle<T> = JoinHandle<T> where T: 'static;

    /// 把 `future` 投递到**本值**的本地队列。
    ///
    /// 队列随本值存活，**不随任务句柄存活**——因此
    /// [`TrJoinHandle::detach`](abs_art::TrJoinHandle::detach) 之后任务仍会被
    /// 持续驱动，直到它自己结束。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        self.local_.spawn(future).into()
    }

    /// 驱动**本值**的本地队列，直到 `future` 完成。
    ///
    /// 返回的 future 需要放在「已处于可驱动环境」的位置 await（smol 没有环境前提，
    /// 典型写法就是 `smol::block_on(value.run_until(fut))`）。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future,
    {
        self.local_.run(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端「运行时值持有本地队列」的单元测试。

    use std::{cell::Cell, rc::Rc};

    use abs_art::{SPAWN_LOCAL, TrJoinHandle, TrLocalScope};

    use crate::Runtime;

    /// 目的：验证 `run_until` 会驱动**本值**的本地队列，`!Send` 任务能跑完。
    ///
    /// 手段：用 `smol::block_on` 驱动 `value.run_until(..)`，在其中投递一个捕获
    /// `Rc<u32>` 的本地任务并 await 其句柄。
    ///
    /// 判定：取回 `6 * 7 == 42`；若队列没有被驱动，await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let value = crate::current();

        let out = smol::block_on(value.run_until(async {
            let rc = Rc::new(6u32);
            value.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证**阻塞**驱动入口 `block_on` 同样会驱动本值的本地队列（值化之后
    /// 「阻塞等待」与「驱动本地队列」是同一个动作）。
    ///
    /// 手段：直接调用 `value.block_on(..)`（内部为 `smol::block_on(local.run(fut))`），
    /// 在其中投递 `!Send` 任务并 await 其句柄。
    ///
    /// 判定：取回 `6 * 7 == 42`；若 `block_on` 漏掉了本地队列，await 会永久挂起。
    #[cfg(feature = "block_on")]
    #[test]
    fn block_on_drives_local_tasks() {
        use abs_art::TrBlockOn;

        let value = crate::current();

        let out = value.block_on(async {
            let rc = Rc::new(6u32);
            value.spawn_local(async move { *rc * 7 }).await.unwrap()
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归运行时值所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 手段：在 `value.run_until` 中投递一个置位 `Rc<Cell<bool>>` 的本地任务后
    /// 立即 `detach()`，再循环 `yield_now` 等标志置位。
    ///
    /// 判定：标志在有限次让出内被置位；若实现把执行器绑在句柄上（detach 即销毁
    /// 执行器 → 任务取消），标志永远不会置位，循环会因超出上限而断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        let value = crate::current();

        smol::block_on(value.run_until(async {
            let flag = Rc::new(Cell::new(false));
            let task_flag = flag.clone();

            let handle = value.spawn_local(async move {
                smol::future::yield_now().await;
                task_flag.set(true);
            });
            handle.detach();

            let mut spins = 0u32;
            while !flag.get() {
                smol::future::yield_now().await;
                spins += 1;
                assert!(spins < 1_000_000, "detach 后本地任务未被推进");
            }
        }));
    }

    /// 目的：验证「声明能力位」路径——只写 `SPAWN_LOCAL` 的运行时**值**确实可用
    /// 本地投递。
    ///
    /// 手段：把 CAPS 写成只含 `SPAWN_LOCAL`，构造值并跑一个捕获 `Rc` 的 `!Send`
    /// 任务。
    ///
    /// 判定：取回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错（例如标在别的位上），
    /// 本测试将无法编译。
    #[test]
    fn declared_cap_gives_usable_local_queue() {
        let value = Runtime::<{ SPAWN_LOCAL }>::current();

        let out = smol::block_on(value.run_until(async {
            let rc = Rc::new(6u32);
            value.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证同一个运行时值的两个克隆共享**同一条**本地队列。
    ///
    /// 手段：克隆值，用克隆体投递任务，用原值驱动，再 await 句柄。
    ///
    /// 判定：取回 5——若两个克隆各有各的队列，驱动原值不会推进克隆体投递的任务，
    /// await 会挂起。
    #[test]
    fn clones_share_one_local_queue() {
        let value = crate::current();
        let clone = value.clone();
        let handle = clone.spawn_local(async { 5u32 });
        let out = smol::block_on(value.run_until(handle)).unwrap();

        assert_eq!(out, 5);
    }

    /// 目的：验证本地投递与 `delay` 在**同一个值**上协同工作（本地队列的驱动循环
    /// 里也能等到 async-io 的计时器）。
    ///
    /// 手段：在 `run_until` 内先 `delay` 1ms，再读墙上时钟的耗时。
    ///
    /// 判定：耗时 ≥ 1ms。
    #[cfg(feature = "delay")]
    #[test]
    fn delay_works_inside_local_driver() {
        use std::time::Duration;

        use abs_art::TrDelay;

        let value = crate::current();
        let start = std::time::Instant::now();
        smol::block_on(value.run_until(async {
            value.delay(Duration::from_millis(1)).await;
        }));
        let elapsed = start.elapsed();

        assert!(elapsed >= Duration::from_millis(1), "耗时为 {elapsed:?}");
    }

    /// 目的：验证 `TrLocalScope::Handle` 与 `crate::JoinHandle` 是同一个类型。
    ///
    /// 手段：把 `spawn_local` 交回的句柄直接传给一个形参类型为
    /// `crate::JoinHandle<u32>` 的函数。
    ///
    /// 判定：编译通过即为通过（类型相等）。
    #[test]
    fn handle_type_is_the_shared_join_handle() {
        fn take_handle_(_: crate::JoinHandle<u32>) {}

        let value = crate::current();
        take_handle_(value.spawn_local(async { 1u32 }));
    }
}
