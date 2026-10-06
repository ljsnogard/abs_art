//! `spawn_send`：把任务投递到 compio 运行时的工作队列。
//!
//! # 「全局队列」在 compio 上是线程本地的
//!
//! [`TrSpawnSend`] 的文档说「投递到**全局（跨线程）**工作队列」——那是 tokio 的
//! 形状。compio 的运行时**本身就是线程本地的**（`compio::runtime::Runtime` 内部
//! 全是 `Rc`，`!Send`，不能跨线程发送），因此本后端如实记录：
//!
//! - [`TrSpawnSend::spawn`] 投递到「**本值抓住的那份运行时**」的工作队列；
//! - trait 要求 `F: Future + Send`（这是「进入全局队列」的声明），compio 的实现
//!   并不需要它，但也不会因此出错——编译器只是替调用方多收一条本可省去的证明；
//! - 于是 [`TrSpawnSend::spawn`] 与 `TrLocalScope::spawn_local` 在本后端是
//!   **同一个入口**（同一个运行时、同一条队列），差别只在 trait 声明的约束上。
//!   这与 tokio（`spawn` 进工作窃取队列、`spawn_local` 进 `LocalSet`）形成对照，
//!   也正是「值化」要暴露的事实：每个后端的前提不同，值如实说出自己的前提。
//!
//! 值化相对 v0.3 的收益：投递打在 `self` 抓住的运行时上，而不是
//! 「当前线程恰好进入了哪个运行时」——后者在同一个进程里存在多套运行时（例如
//! 测试二进制）时会静默投错。

use core::future::Future;

use abs_art::{HasSpawnSend, TrSpawnSend};

use crate::{Runtime, join_handle::JoinHandle};

impl<const CAPS: usize> TrSpawnSend for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnSend,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    /// 把 `future` 投递到**本值抓住的** compio 运行时的工作队列。
    ///
    /// 不要求调用点处于 compio 运行时上下文内：句柄已经在值里了。
    /// `Send` 约束来自 trait 的跨线程声明，compio 侧并不需要它。
    fn spawn<F>(&self, future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        self.rt_.spawn(future).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `spawn_send` 功能单元测试。

    use abs_art::{TrBlockOn, TrSpawnSend};
    use compio::runtime::Runtime as CompioRuntime;

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn`（值方法）能把 future 投递到该值抓住的 compio
    /// 运行时，并通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建 compio 运行时，在 `rt.block_on` 中构造运行时值并调用其
    /// `spawn`，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn spawn_returns_output() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let handle = value.spawn(async { 6 * 7 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证值化之后 `spawn` 打的是**值抓住的**运行时，而不是环境运行时
    /// ——在 compio 上下文之外用 `with_runtime` 搬进来的值同样能投递、能驱动。
    ///
    /// 实施策略：在测试线程（无 compio 上下文）用 `Runtime::with_runtime` 构造值，
    /// 再 `value.block_on` 驱动一个先 `spawn` 再 await 的 async 块。
    ///
    /// 通过依据：取回 40 + 2 == 42；若实现仍走 `with_current`，构造/投递会 panic。
    #[test]
    fn spawn_uses_the_value_runtime_outside_context() {
        let rt = CompioRuntime::new().unwrap();
        let value = Runtime::<{ crate::FULL }>::with_runtime(rt.clone());

        let out = value.block_on(async {
            let handle = value.spawn(async { 40 + 2 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
