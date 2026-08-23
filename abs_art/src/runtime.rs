//! 抽象运行时标签与能力 trait。
//!
//! 本模块不依赖任何异步运行时，所有 trait 的具体实现都在组合 crate
//! （`abs_art-tokio` / `abs_art-compio` / `abs_art-smol`）中给出。

use core::future::Future;

/// 抽象运行时标签。
///
/// 每个组合 crate 对应一个具体的变体（例如 [`abs_art_tokio`] 对应
/// [`Runtime::Tokio`]），本 crate 本身不实现任何运行时行为。
///
/// [`abs_art_tokio`]: https://docs.rs/abs_art-tokio
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Runtime {
    /// compio 运行时。
    Compio,
    /// smol 运行时。
    Smol,
    /// tokio 运行时。
    Tokio,
}

pub trait TrAsyncRuntime {
    type JoinHandle<T>: TrJoinHandle<T> where T: 'static;

    fn about() -> Runtime;
}

pub trait TrJoinHandle<T>
where
    Self: Future<Output = Result<T, Self::JoinErr>>
{
    type JoinErr: core::error::Error;
}

/// 运行时可以把任务投递到全局（跨线程）工作窃取队列。
///
/// `H` 是返回的任务句柄类型，由组合 crate 给出（例如 `abs_art-tokio`
/// 中的 [`JoinHandle`]）。把句柄类型做成泛型参数，是为了让本基础 crate
/// 不持有任何与具体运行时相关的类型。
///
/// [`JoinHandle`]: https://docs.rs/abs_art-tokio
pub trait TrSpawnSend<F>
where
    F: Future + Send + 'static,
    <F as Future>::Output: Send + 'static,
{
    type JoinHandle<T>: TrJoinHandle<T> where T: 'static;

    /// 把 `future` 投递到全局工作队列，返回句柄 `H`。
    fn spawn(future: F) -> Self::JoinHandle<<F as Future>::Output>;
}

/// 运行时可以把任务投递到线程本地工作队列。
pub trait TrSpawnLocal<F>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
{
    type JoinHandle<T>: TrJoinHandle<T> where T: 'static;

    /// 把 `future` 投递到线程本地工作队列，返回句柄 `H`。
    fn spawn_local(future: F) -> Self::JoinHandle<<F as Future>::Output>;
}

/// 运行时可以把阻塞函数投递到阻塞线程池。
pub trait TrSpawnBlocking<F, T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    type JoinHandle: TrJoinHandle<T> where T: 'static;

    /// 把阻塞函数 `f` 投递到阻塞线程池，返回句柄 `H`。
    fn spawn_blocking(f: F) -> Self::JoinHandle;
}

/// 让当前线程阻塞等待一个异步任务完成，同时不影响运行时的调度。
pub trait TrBlockOn<F>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
{
    /// 阻塞当前线程，等待 `f` 完成并返回其结果。
    fn block_on(f: F) -> F::Output;
}

/// 暂停当前执行上下文一段时间。
pub trait TrDelay {
    /// 返回一个等待 `duration` 之后完成的 future。
    fn delay(duration: core::time::Duration) -> impl Future<Output = ()>;
}
