//! `local_scope`：值化的本地作用域——本地队列（`LocalSet`）的持有者与驱动点。
//!
//! # 为什么是这个形状
//!
//! tokio 的 `spawn_local` 有三条硬约束：
//!
//! 1. `tokio::task::spawn_local`（自由函数）**必须在 `LocalSet` 上下文内**调用，
//!    否则 panic——纯类型参数表达不了这条环境前提；
//! 2. 本地队列归调用方的 [`LocalSet`] 所有，**必须由调用方驱动**；
//! 3. `LocalSet` 是 `!Send` 的，绑定创建它的线程。
//!
//! 因此本后端提供 [`LocalScope`]：它就是那个 `LocalSet` 的持有者，同时是投递点与
//! 驱动点。投递走**方法版** [`LocalSet::spawn_local`]——它在 `LocalSet` 未运行时
//! 也能投递且不 panic，正是「先建作用域、后驱动」这个用法需要的语义。

use alloc::rc::Rc;
use core::future::Future;

use abs_art::{HasSpawnLocal, TrLocalScope};
use tokio::task::LocalSet;

use crate::{Runtime, join_handle::JoinHandle};

/// 值化的本地作用域（tokio 后端）。
///
/// 内部持有一个 `Rc<LocalSet>`：本地队列随本值存活，**不随任务句柄存活**——
/// 因此 [`TrJoinHandle::detach`](abs_art::TrJoinHandle::detach) 之后任务仍会被
/// 持续驱动，直到它自己结束。克隆本值即共享同一条本地队列。
pub struct LocalScope {
    local_: Rc<LocalSet>,
}

impl LocalScope {
    /// 创建本地作用域（内部新建一条本地队列）。
    ///
    /// `LocalSet` 绑定创建它的线程，因此本值以及投递到它上面的所有任务，
    /// 都只在当前线程上运行。
    ///
    /// 这是**不经声明**的直接入口，集成方常用。业务库若要显式声明依赖，应改用
    /// [`Runtime::local_scope`]（它会要求能力位含 `SPAWN_LOCAL`）。
    pub fn new() -> Self {
        Self {
            local_: Rc::new(LocalSet::new()),
        }
    }
}

impl<const CAPS: usize> Runtime<CAPS>
where
    [(); CAPS]: HasSpawnLocal,
{
    /// 声明式地取得本地作用域（「声明 → 取得」的串联点）。
    ///
    /// # 为什么有这个入口
    ///
    /// [`SPAWN_LOCAL`](abs_art::SPAWN_LOCAL) 能力位是一句**写在代码上的声明**：
    /// 它拦不住真想用本地投递的人，但强制他把这件事写下来，于是这次「升级」必然
    /// 出现在类型别名、diff 与 code review 里。本关联函数要求 `CAPS` 含该位——
    /// 想经它拿到作用域值，就得先写下那位。
    ///
    /// 注意这不是安全边界：[`LocalScope::new`] 仍是公开入口，绕过声明依然可行。
    /// 声明位的价值是「**必须写下来**」，不是「写不下来就用不了」。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::TrLocalScope;
    /// use abs_art_tokio::{Runtime, SPAWN_LOCAL};
    ///
    /// let rt = tokio::runtime::Builder::new_current_thread()
    ///     .build()
    ///     .unwrap();
    ///
    /// // 声明了 SPAWN_LOCAL，才能经这个入口取得作用域
    /// let scope = Runtime::<{ SPAWN_LOCAL }>::local_scope();
    ///
    /// let out = rt.block_on(scope.run_until(async {
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
            local_: Rc::clone(&self.local_),
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
    /// 走 [`LocalSet::spawn_local`]（方法版）而非自由函数：方法版**不要求**
    /// `LocalSet` 正在运行，也不需要进入 `LocalSet` 上下文，因此不会 panic。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        self.local_.spawn_local(future).into()
    }

    /// 驱动本地队列直到 `future` 完成。
    ///
    /// 需要外层已有一个 tokio 运行时在驱动本 future——典型写法是
    /// `rt.block_on(scope.run_until(fut))`。
    fn run_until<F>(&self, future: F) -> impl Future<Output = F::Output>
    where
        F: Future,
    {
        self.local_.run_until(future)
    }

    /// 阻塞当前线程，驱动本地队列直到 `future` 完成。
    ///
    /// 与 abs_art-tokio 的 [`TrBlockOn`](abs_art::TrBlockOn) 前提一致：
    /// **多线程运行时**且调用点已处于运行时上下文内。实现先经
    /// `block_in_place` 让渡当前 worker，再用 `Handle::block_on` 驱动
    /// [`run_until`](Self::run_until)——于是等待期间本地队列持续被推进。
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.local_.run_until(future))
        })
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端 [`LocalScope`] 的单元测试。

    use std::{cell::Cell, rc::Rc};

    use abs_art::{TrJoinHandle, TrLocalScope};

    use crate::LocalScope;

    /// 目的：验证 `run_until` 在等待传入 future 期间持续驱动本地队列，且结果能经
    /// 句柄取回。
    ///
    /// 实施策略：用 current_thread 运行时驱动 `scope.run_until(..)`，在其中投递一个
    /// 捕获 `Rc` 的 `!Send` 任务并 await 其句柄。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若 `run_until` 没有驱动本地队列，await 会永久
    /// 挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let out = rt.block_on(async {
            let scope = LocalScope::new();
            scope
                .run_until(async {
                    let rc = Rc::new(6u32);
                    let handle = scope.spawn_local(async move { *rc * 7 });
                    handle.await.unwrap()
                })
                .await
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归作用域所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 实施策略：在多线程运行时上下文内用 `scope.block_on` 驱动；投递一个置位
    /// `Rc<Cell<bool>>` 的本地任务后立即 `detach()`，再循环 `yield_now` 等标志置位。
    ///
    /// 通过依据：标志在有限次让出内被置位；若实现把队列绑在句柄上（drop 即取消），
    /// 循环会因超出上限而断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let scope = LocalScope::new();
            scope.block_on(async {
                let flag = Rc::new(Cell::new(false));
                let task_flag = flag.clone();

                let handle = scope.spawn_local(async move {
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

    /// 目的：验证「声明 → 取得」路径——`Runtime::<{ SPAWN_LOCAL }>::local_scope()`
    /// 取得的作用域确实可用。
    ///
    /// 实施策略：只声明 `SPAWN_LOCAL` 一位的 `Runtime` 上调 `local_scope()`，再用
    /// `run_until` 驱动一个捕获 `Rc` 的 `!Send` 任务。
    ///
    /// 通过依据：交回 `6 * 7 == 42`；若 `HasSpawnLocal` 的门控写错（例如标在别的
    /// 位上），本测试将无法编译。
    #[test]
    fn declared_cap_gives_usable_scope() {
        use abs_art::SPAWN_LOCAL;

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let scope = crate::Runtime::<{ SPAWN_LOCAL }>::local_scope();

        let out = rt.block_on(scope.run_until(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }
}
