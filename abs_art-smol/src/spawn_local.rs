//! `spawn_local`：把任务投递到线程本地执行器。
//!
//! smol 2.x 本身没有内建的 `spawn_local`（它只提供全局执行器的 `spawn`）。
//! 这里用 `async_executor::LocalExecutor`（smol 会 re-export 为
//! [`smol::LocalExecutor`]）实现：每次调用创建一个本地执行器并投递任务，
//! 执行器随返回的 [`JoinHandle`](crate::JoinHandle) 一起存活。
//!
//! 调度说明：smol 2.x 的 `block_on`（`async_io::block_on`）只轮询给定的
//! future，不会驱动本地执行器。因此本实现让 [`JoinHandle`](crate::JoinHandle)
//! 在每次 poll 时驱动它持有的本地执行器，使被 join 的本地任务得以推进。
//! 限制：未被 join 的本地任务在句柄被 drop 时随执行器一起取消；本地任务内部
//! 若还有需要外部驱动的异步等待（如 timer），可能无法推进。

use core::future::Future;

use crate::{join_handle::JoinHandle, Runtime};
use abs_art::{FULL, HasSpawnLocal, TrSpawnLocal};

impl Runtime<FULL> {
    /// 把 `future` 投递到线程本地执行器，返回 [`JoinHandle`]。
    pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
    where
        Self: TrSpawnLocal<F>,
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        let ex = smol::LocalExecutor::new();
        let task = ex.spawn(future);
        JoinHandle::from_local(task, ex)
    }
}

impl<F, const CAPS: usize> TrSpawnLocal<F> for Runtime<CAPS>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
    [(); CAPS]: HasSpawnLocal,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    fn spawn_local(future: F) -> JoinHandle<F::Output> {
        Runtime::spawn_local(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `spawn_local` 功能单元测试。

    use crate::Runtime;

    /// 目的：验证 `Runtime::spawn_local` 能投递一个非 `Send` 的任务，并通过
    /// 返回的 [`JoinHandle`](crate::JoinHandle)（poll 时驱动本地执行器）取回
    /// 结果。
    ///
    /// 实施策略：在 `smol::block_on` 中调用 `Runtime::spawn_local` 投递一个
    /// 捕获了非 `Send` 数据（`Rc`）的 future，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(6 * 7 == 42)`；若实现错误地要求任务
    /// `Send`，编译会失败（从而测试无法通过）；若本地执行器没有被驱动，await
    /// 将永远无法完成（测试挂死）。
    #[test]
    fn spawn_local_returns_output() {
        let out = smol::block_on(async {
            let rc = std::rc::Rc::new(6);
            let handle = Runtime::spawn_local(async move {
                let a = *rc;
                let b = 7;
                a * b
            });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }
}
