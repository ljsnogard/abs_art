//! `local_scope`：本地队列由**运行时值**持有——投递点与驱动点都是这个值。
//!
//! # 为什么是这个形状
//!
//! tokio 的 `spawn_local` 有三条硬约束：
//!
//! 1. 本地任务必须投递到**某一条** `LocalSet` 上；
//! 2. 本地队列归 `LocalSet` 的持有者所有，**必须由持有者驱动**；
//! 3. `LocalSet` 是 `!Send` 的，绑定创建它的线程。
//!
//! v0.3 把队列做成一个**独立的作用域值**（`LocalScope`），于是「驱动谁」与
//! 「用哪个运行时」是两件事，调用方要分别记住。v0.4 把它并回 [`Runtime`]：
//! 值本身就是队列的持有者与驱动点，
//!
//! - 投递走方法版 [`LocalSet::spawn_local`]——它在 `LocalSet` 未运行时也能投递且
//!   不 panic，正是「先建队列、后驱动」这个用法需要的语义；
//! - 驱动走 [`TrLocalScope::run_until`]（异步）或
//!   [`TrBlockOn::block_on`](abs_art::TrBlockOn::block_on)（阻塞，见 `block_on.rs`）；
//! - 克隆运行时值即共享**同一条**队列（`Rc<LocalSet>`）。

use core::future::Future;

use abs_art::{HasSpawnLocal, TrLocalScope};

use crate::{JoinHandle, Runtime};

impl<const CAPS: usize> TrLocalScope for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnLocal,
{
    /// 本后端的本地任务句柄：与全局 `spawn` 共用同一个 [`JoinHandle`]。
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
        self.local_.spawn_local(future).into()
    }

    /// 驱动**本值**的本地队列，直到 `future` 完成。
    ///
    /// 返回的 future 需要放在「已处于该 tokio 运行时上下文」的位置 await
    /// （典型：`rt.block_on(value.run_until(fut))`）。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future,
    {
        self.local_.run_until(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端「运行时值持有本地队列」的单元测试。

    use std::{cell::Cell, rc::Rc, time::Duration};

    use abs_art::{SPAWN_LOCAL, TrDelay, TrJoinHandle, TrLocalScope};

    use crate::Runtime;

    /// 目的：验证 `run_until` 会驱动**本值**的本地队列，`!Send` 任务能跑完。
    ///
    /// 实施策略：在 current_thread tokio 运行时里构造运行时值，投递一个捕获
    /// `Rc<u32>` 的本地任务，用 `run_until` 驱动并 await 其句柄。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若队列没有被驱动，await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            value
                .run_until(async {
                    let rc = Rc::new(6u32);
                    let handle = value.spawn_local(async move { *rc * 7 });
                    handle.await.unwrap()
                })
                .await
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归运行时值所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 实施策略：在多线程运行时上下文内用 `value.block_on` 驱动；投递一个置位
    /// `Rc<Cell<bool>>` 的本地任务后立即 `detach()`，再循环 `yield_now` 等标志置位。
    ///
    /// 通过依据：标志在有限次让出内被置位；若实现把队列绑在句柄上（drop 即取消），
    /// 循环会因超出上限而断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        use abs_art::TrBlockOn;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let value = crate::current();
            value.block_on(async {
                let flag = Rc::new(Cell::new(false));
                let task_flag = Rc::clone(&flag);

                let handle = value.spawn_local(async move {
                    tokio::task::yield_now().await;
                    task_flag.set(true);
                });
                handle.detach();

                let mut spins = 0u32;
                while !flag.get() {
                    tokio::task::yield_now().await;
                    spins += 1;
                    assert!(spins < 1_000_000, "detach 后本地任务未被推进");
                }
            });
        });
    }

    /// 目的：验证「声明能力位」路径——只写 `SPAWN_LOCAL` 的运行时值确实可用本地投递。
    ///
    /// 实施策略：把 CAPS 写成只含 `SPAWN_LOCAL`，在 tokio 运行时里构造值并跑一个
    /// 捕获 `Rc` 的 `!Send` 任务。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错（例如标在别的
    /// 位上），本测试将无法编译。
    #[test]
    fn declared_cap_gives_usable_local_queue() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

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

    /// 目的：验证同一个运行时值的两个克隆共享**同一条**本地队列。
    ///
    /// 实施策略：克隆值，用克隆体投递任务，用原值驱动，再 await 句柄。
    ///
    /// 通过依据：取回 5——若两个克隆各有各的队列，驱动原值不会推进克隆体投递的
    /// 任务，await 会挂起。
    #[test]
    fn clones_share_one_local_queue() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let clone = value.clone();
            let handle = clone.spawn_local(async { 5u32 });
            value.run_until(handle).await.unwrap()
        });

        assert_eq!(out, 5);
    }

    /// 目的：验证本地投递与 `delay` 在**同一个值**上协同工作。
    ///
    /// 实施策略：在 `run_until` 内先 `delay` 1ms，再读墙上时钟的耗时。
    ///
    /// 通过依据：耗时 ≥ 1ms。
    #[test]
    fn delay_works_inside_local_driver() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let elapsed = rt.block_on(async {
            let value = crate::current();
            let start = std::time::Instant::now();
            value
                .run_until(async {
                    value.delay(Duration::from_millis(1)).await;
                })
                .await;
            start.elapsed()
        });

        assert!(elapsed >= Duration::from_millis(1), "耗时为 {elapsed:?}");
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

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let value = crate::current();
            take_handle_(value.spawn_local(async { 1u32 }));
        });
    }
}
