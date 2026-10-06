//! `spawn_blocking`：把阻塞函数投递到 smol 的阻塞线程池。
//!
//! 底层是 `smol::unblock`（即 `blocking::unblock`）：它把闭包交给 `blocking` 的
//! **进程级**线程池执行，因此与 `spawn` 一样，这条能力**不归运行时值所有**——
//! 线程池由 `blocking` 懒初始化并常驻，任何值都持不了、也销毁不了它。可观测后果：
//!
//! 1. 值被 drop 不会停止已投递的阻塞任务；
//! 2. 线程池的规模是进程级配置（`blocking` 自己按需增减线程），不是本值的能力；
//! 3. 因此被投递的闭包**不会**在调用线程上运行（这一点由本模块的单测钉住）。
//!
//! 值化在这里只提供**统一的调用形状**（业务库写 `rt.spawn_blocking(..)`），
//! 与 `abs_art-tokio` 的 `Handle::spawn_blocking` 在调用点上完全一致。

use abs_art::{HasSpawnBlocking, TrSpawnBlocking};

use crate::{Runtime, join_handle::JoinHandle};

impl<const CAPS: usize> TrSpawnBlocking for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnBlocking,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    /// 把阻塞函数 `f` 投递到 `blocking` 的进程级线程池，返回句柄。
    ///
    /// `&self` 在实现里不被读取——理由见模块文档（阻塞线程池是进程级资源）。
    fn spawn_blocking<F, T>(&self, f: F) -> Self::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        smol::unblock(f).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `spawn_blocking` 功能单元测试。

    use abs_art::{FULL, TrSpawnBlocking};

    use crate::Runtime;

    /// 目的：验证运行时值的 `spawn_blocking` 能把阻塞函数投递到阻塞线程池，并通过
    /// 返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 手段：构造 `Runtime<{ FULL }>` 值，调用其 `spawn_blocking` 执行一个简单的
    /// 同步计算，在 `smol::block_on` 里 await 返回的句柄。
    ///
    /// 判定：句柄结果为 `Ok(40 + 2 == 42)`。
    #[test]
    fn spawn_blocking_returns_output() {
        let value = Runtime::<{ FULL }>::current();
        let out = smol::block_on(async {
            let handle = value.spawn_blocking(|| 40 + 2);
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证被投递的闭包**不在调用线程上运行**——即它真的被交给了阻塞线程池，
    /// 而不是在 `block_on` 的轮询里就地执行（后者会把异步线程堵死）。
    ///
    /// 手段：记录调用线程的 `ThreadId`，在闭包里记录执行线程的 `ThreadId`，把两者
    /// 一起带回来比较。
    ///
    /// 判定：执行线程的 id **不等于**调用线程的 id；若相等，说明实现退化为同步调用。
    #[test]
    fn spawn_blocking_runs_off_the_calling_thread() {
        let value = Runtime::<{ FULL }>::current();
        let caller = std::thread::current().id();

        let worker = smol::block_on(async {
            value
                .spawn_blocking(move || std::thread::current().id())
                .await
                .unwrap()
        });

        assert_ne!(caller, worker, "阻塞闭包不该在调用线程上执行");
    }
}
