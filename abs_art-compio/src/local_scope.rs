//! `local_scope`：值化的本地作用域——compio 后端，**零大小**。
//!
//! # 为什么这里可以是一个空类型
//!
//! compio 的运行时**本身就是线程本地的**（`compio::runtime::Runtime` 是 `!Send`，
//! 不能跨线程发送），因此：
//!
//! - `Runtime::spawn` **不要求** `F: Send`，`spawn` 与「本地投递」是同一条队列、
//!   同一套语义；
//! - 这条队列归运行时所有，并由运行时自己在 `block_on` / `wait` 期间驱动。
//!
//! 也就是说：**调用方不需要提供任何东西**。空类型不是占位符，而是如实表达这一点
//! ——这与 tokio（必须由调用方提供 `LocalSet`）和 smol（必须自建 `LocalExecutor`）
//! 形成了对照，也正是「值化」的意义：每个后端如实说出自己的前提，需要什么就装
//! 什么，不需要就是空的。

use core::future::Future;

use abs_art::{HasSpawnLocal, TrLocalScope};
use compio::runtime::Runtime as CompioRuntime;

use crate::{Runtime, join_handle::JoinHandle};

/// 值化的本地作用域（compio 后端，零大小）。
///
/// 无状态、可自由复制；它存在只是为了满足统一的 [`TrLocalScope`] 契约。
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalScope;

impl LocalScope {
    /// 创建本地作用域（无状态）。
    ///
    /// 这是**不经声明**的直接入口。业务库若要显式声明依赖，应改用
    /// [`Runtime::local_scope`]（它会要求能力位含 `SPAWN_LOCAL`）。
    pub fn new() -> Self {
        Self
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
    /// use abs_art_compio::{Runtime, SPAWN_LOCAL};
    ///
    /// let rt = compio::runtime::Runtime::new().unwrap();
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

impl TrLocalScope for LocalScope {
    type Handle<T>
        = JoinHandle<T>
    where
        T: 'static;

    /// 投递到当前 compio 运行时的队列（线程本地，因此不需要 `Send`）。
    ///
    /// 必须在 compio 运行时上下文内调用；否则 `Runtime::with_current` 会 panic。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        CompioRuntime::with_current(|rt| rt.spawn(future)).into()
    }

    /// 驱动本地队列直到 `future` 完成。
    ///
    /// compio 的队列由运行时自己驱动，因此「驱动直到 `future` 完成」就是
    /// `future` 本身——外层只需要一个 compio 的 `block_on`（或 `wait`）在跑。
    fn run_until<F>(&self, future: F) -> impl Future<Output = F::Output>
    where
        F: Future,
    {
        future
    }

    /// 阻塞当前线程，驱动本地队列直到 `future` 完成。
    ///
    /// 与 abs_art-compio 的 [`TrBlockOn`](abs_art::TrBlockOn) 前提一致：
    /// 需要已处于 compio 运行时上下文内。
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        CompioRuntime::with_current(|rt| rt.block_on(future))
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端 [`LocalScope`] 的单元测试。

    use std::{cell::Cell, rc::Rc, time::Duration};

    use compio::runtime::Runtime as CompioRuntime;

    use abs_art::{TrJoinHandle, TrLocalScope};

    use crate::LocalScope;

    /// 目的：验证 `run_until` / `block_on` 能驱动 compio 的本地队列，且结果能经句柄
    /// 取回。
    ///
    /// 实施策略：创建 compio 运行时，用 `rt.block_on(scope.run_until(..))` 驱动一个
    /// 投递了 `!Send`（捕获 `Rc`）任务并 await 其句柄的 future。
    ///
    /// 通过依据：取回 `6 * 7 == 42`；若本地队列没有被驱动，await 会永久挂起。
    #[test]
    fn run_until_drives_local_tasks() {
        let rt = CompioRuntime::new().unwrap();
        let scope = LocalScope::new();

        let out = rt.block_on(scope.run_until(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }

    /// 目的：验证本地队列归运行时所有——`detach()` 消费句柄后任务仍继续运行。
    ///
    /// 实施策略：投递一个置位 `Rc<Cell<bool>>` 的本地任务后立即 `detach()`，再循环
    /// `sleep` 等标志置位（带 1 秒期限）。
    ///
    /// 通过依据：标志在期限内被置位；若 detach 实际取消了任务，断言失败。
    #[test]
    fn detach_keeps_local_task_running() {
        let rt = CompioRuntime::new().unwrap();
        let scope = LocalScope::new();

        rt.block_on(scope.run_until(async {
            let flag = Rc::new(Cell::new(false));
            let task_flag = flag.clone();

            let handle = scope.spawn_local(async move {
                task_flag.set(true);
            });
            handle.detach();

            let mut elapsed = 0u32;
            while !flag.get() && elapsed < 1_000 {
                compio::runtime::time::sleep(Duration::from_millis(1)).await;
                elapsed += 1;
            }
            assert!(flag.get(), "detach 后本地任务未被推进");
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

        let rt = CompioRuntime::new().unwrap();
        let scope = crate::Runtime::<{ SPAWN_LOCAL }>::local_scope();

        let out = rt.block_on(scope.run_until(async {
            let rc = Rc::new(6u32);
            scope.spawn_local(async move { *rc * 7 }).await.unwrap()
        }));

        assert_eq!(out, 42);
    }
}
