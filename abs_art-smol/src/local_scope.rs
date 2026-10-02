//! `local_scope`：值化的本地作用域——smol 后端。
//!
//! # 为什么必须由 abs_art 提供这个值
//!
//! smol 2.x **没有**内建的 `spawn_local`：它只提供全局执行器（`smol::spawn`）。
//! 本地队列必须由使用方用 `async_executor::LocalExecutor`（smol 重导出为
//! [`smol::LocalExecutor`]）自建，并且**自己驱动**（`LocalExecutor::run` / `tick`）。
//!
//! v0.3 之前的实现把执行器塞进 `JoinHandle`、在句柄被 poll 时顺带 tick 它。那让
//! 「本地任务能否推进」变成了「调用方有没有在 poll 句柄」的函数，而 `detach()`
//! 消费句柄会连带销毁执行器，**本地任务当场被取消**。
//!
//! 现在执行器归 [`LocalScope`] 所有：句柄只持有 `smol::Task`，任务能否推进与句柄
//! 无关，`detach()` 也回到正常语义。

use alloc::rc::Rc;
use core::future::Future;

use abs_art::{HasSpawnLocal, TrLocalScope};
use smol::LocalExecutor;

use crate::{Runtime, join_handle::JoinHandle};

/// 值化的本地作用域（smol 后端）。
///
/// 内部持有一个 `Rc<LocalExecutor<'static>>`：本地队列随本值存活，**不随任务句柄
/// 存活**。克隆本值即共享同一条本地队列。
///
/// 注意本地队列是**线程本地**的，因此本值以及投递到它上面的任务都只在当前线程
/// 上运行。
pub struct LocalScope {
    ex_: Rc<LocalExecutor<'static>>,
}

impl LocalScope {
    /// 创建本地作用域（内部新建一条线程本地队列）。
    ///
    /// 这是**不经声明**的直接入口。业务库若要显式声明依赖，应改用
    /// [`Runtime::local_scope`]（它会要求能力位含 `SPAWN_LOCAL`）。
    pub fn new() -> Self {
        Self {
            ex_: Rc::new(LocalExecutor::new()),
        }
    }
}

impl<const CAPS: usize> Runtime<CAPS>
where
    [(); CAPS]: HasSpawnLocal,
{
    /// 声明式地取得本地作用域（「声明 → 取得」的串联点）。
    ///
    /// [`SPAWN_LOCAL`](abs_art::SPAWN_LOCAL) 能力位是一句**写在代码上的声明**：
    /// 本关联函数要求 `CAPS` 含该位，于是「我要用本地投递」这件事必须先被写下来，
    /// 才能经它拿到作用域值。
    ///
    /// 注意这不是安全边界：[`LocalScope::new`] 仍是公开入口，绕过声明依然可行。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::TrLocalScope;
    /// use abs_art_smol::{Runtime, SPAWN_LOCAL};
    ///
    /// // 声明了 SPAWN_LOCAL，才能经这个入口取得作用域
    /// let scope = Runtime::<{ SPAWN_LOCAL }>::local_scope();
    ///
    /// let out = smol::block_on(scope.run_until(async {
    ///     let rc = std::rc::Rc::new(6u32);
    ///     scope.spawn_local(async move { *rc * 7 }).await.unwrap()
    /// }));
    /// assert_eq!(out, 42);
    /// ```
    pub fn local_scope() -> LocalScope {
        LocalScope::new()
    }
}

impl Default for LocalScope {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for LocalScope {
    fn clone(&self) -> Self {
        Self {
            ex_: Rc::clone(&self.ex_),
        }
    }
}

impl core::fmt::Debug for LocalScope {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocalScope").finish_non_exhaustive()
    }
}

impl TrLocalScope for LocalScope {
    type Handle<T>
        = JoinHandle<T>
    where
        T: 'static;

    /// 投递到本作用域的本地队列。
    ///
    /// 句柄不持有执行器——队列归本作用域所有（这正是 `detach` 之后任务还能继续
    /// 推进的原因）。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        self.ex_.spawn(future).into()
    }

    /// 驱动本地队列直到 `future` 完成。
    ///
    /// 底层是 `LocalExecutor::run`（「驱动执行器直到给定 future 完成」），因此
    /// 典型写法是 `smol::block_on(scope.run_until(fut))`。
    fn run_until<F>(&self, future: F) -> impl Future<Output = F::Output>
    where
        F: Future,
    {
        self.ex_.run(future)
    }

    /// 阻塞当前线程，驱动本地队列直到 `future` 完成。
    ///
    /// smol 的 `block_on` 不依赖任何「环境运行时」，因此本入口**没有先决条件**。
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        smol::block_on(self.ex_.run(future))
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端 [`LocalScope`] 的单元测试。

    use std::{cell::Cell, rc::Rc};

    use abs_art::{TrJoinHandle, TrLocalScope};

    use crate::LocalScope;

    /// 目的：验证 `run_until` 在等待传入 future 期间持续驱动本地队列，且结果能经
    /// 句柄取回。
    ///
    /// 实施策略：用 `smol::block_on` 驱动 `scope.run_until(..)`，在其中投递一个
    /// 捕获 `Rc` 的 `!Send` 任务并 await 其句柄。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `run_until` 没有驱动本地队列，await 会永久
    /// 挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let scope = LocalScope::new();

        let out = smol::block_on(scope.run_until(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证 `block_on` 便捷入口同样能驱动本地队列。
    ///
    /// 实施策略：直接调用 `scope.block_on(..)`（内部为 `smol::block_on(ex.run(fut))`），
    /// 在其中投递 `!Send` 任务并 await 其句柄。
    ///
    /// 通过依据：取回 `6 * 7 == 42`。
    #[test]
    fn block_on_drives_local_tasks() {
        let scope = LocalScope::new();

        let out = scope.block_on(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归作用域所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 实施策略：在 `scope.run_until` 中投递一个置位 `Rc<Cell<bool>>` 的本地任务后
    /// 立即 `detach()`，再循环 `yield_now` 等标志置位。
    ///
    /// 通过依据：标志在有限次让出内被置位；若实现仍把执行器绑在句柄上（detach 即
    /// 销毁执行器 → 任务取消），标志永远不会置位，循环会因超出上限而断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        let scope = LocalScope::new();

        smol::block_on(scope.run_until(async {
            let flag = Rc::new(Cell::new(false));
            let task_flag = flag.clone();

            let handle = scope.spawn_local(async move {
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

    /// 目的：验证「声明 → 取得」路径——`Runtime::<{ SPAWN_LOCAL }>::local_scope()`
    /// 取得的作用域确实可用。
    ///
    /// 实施策略：只声明 `SPAWN_LOCAL` 一位的 `Runtime` 上调 `local_scope()`，再用
    /// `run_until` 驱动一个捕获 `Rc` 的 `!Send` 任务。
    ///
    /// 通过依据：交回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错，本测试无法编译。
    #[test]
    fn declared_cap_gives_usable_scope() {
        use abs_art::SPAWN_LOCAL;

        let scope = crate::Runtime::<{ SPAWN_LOCAL }>::local_scope();

        let out = smol::block_on(scope.run_until(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }
}
