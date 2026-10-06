//! `spawn_blocking`：把阻塞函数投递到 compio 的阻塞线程池。
//!
//! 值化之后投递打在 `self` 抓住的那份运行时上：compio 的
//! `Runtime::spawn_blocking` 内部把闭包包成 `Asyncify` 操作提交给**这份运行时**的
//! 驱动，再把等待它的 future `spawn` 到同一个执行器，因此既不需要 compio 运行时
//! 上下文，也不会投错运行时。

use abs_art::{HasSpawnBlocking, TrSpawnBlocking};

use crate::{CompioCaps_, Runtime, join_handle::JoinHandle};

impl<const CAPS: usize> TrSpawnBlocking for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnBlocking,
    [(); CAPS]: CompioCaps_,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    /// 把阻塞函数 `f` 投递到**本值抓住的** compio 运行时的阻塞线程池。
    ///
    /// compio 侧在单独的线程上执行 `f`（`Asyncify`），完成后再唤醒等待它的
    /// future；`f` 因此需要 `Send + 'static`，与 trait 的约束一致。
    fn spawn_blocking<F, T>(&self, f: F) -> Self::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.rt_.spawn_blocking(f).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `spawn_blocking` 功能单元测试。

    use abs_art::TrSpawnBlocking;
    use compio::runtime::Runtime as CompioRuntime;

    /// 目的：验证 `Runtime::spawn_blocking`（值方法）能把阻塞函数投递到 compio 的
    /// 阻塞线程池，并通过返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 实施策略：创建 compio 运行时，在 `rt.block_on` 中构造运行时值并调用其
    /// `spawn_blocking` 执行一个简单的同步计算，await 其 JoinHandle。
    ///
    /// 通过依据：JoinHandle 结果为 `Ok(40 + 2 == 42)`。
    #[test]
    fn spawn_blocking_returns_output() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let handle = value.spawn_blocking(|| 40 + 2);
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证阻塞函数确实跑在**别的线程**上（compio 用 `Asyncify` 把它交给
    /// 阻塞线程池），而不是内联在轮询点执行——否则「阻塞函数不卡住运行时」这条
    /// 承诺就是空的。
    ///
    /// 实施策略：在运行时值上 `spawn_blocking` 一个返回
    /// `std::thread::current().id()` 的闭包，与调用点所在线程的 id 比较。
    ///
    /// 通过依据：两个线程 id 不相等；若实现退化成内联执行，两者会相等。
    #[test]
    fn spawn_blocking_runs_off_the_calling_thread() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let here = std::thread::current().id();
            let handle = value.spawn_blocking(move || std::thread::current().id());
            (here, handle.await.unwrap())
        });

        assert_ne!(out.0, out.1, "阻塞函数不该在调用线程上内联执行");
    }
}
