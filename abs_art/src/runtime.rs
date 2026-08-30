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

    /// 让任务脱离句柄，继续在后台运行，不再能 join / await 其结果。
    ///
    /// `detach` 消费掉句柄本身：调用后任务仍在运行时的队列里继续推进，
    /// 但调用方失去了等待它完成的能力（任务完成后其输出会被丢弃）。
    ///
    /// # 三个后端的支持情况（可行性结论）
    ///
    /// - **tokio**：无原生 `detach` 方法（`JoinHandle` 只有 `abort` /
    ///   `is_finished` / `abort_handle` / `id`）；官方文档明确「drop 句柄
    ///   即 detach」——任务继续在后台运行。实现为丢弃句柄即可，语义正确。
    /// - **smol**：有原生 `Task::detach`（底层 async-task：置 detached 标志
    ///   后 forget）。**不能**靠 drop 实现：async-task 的 `Task` 在 drop 时
    ///   会 `set_canceled()` 取消任务。
    /// - **compio**：有原生 `JoinHandle::detach`（丢弃任务句柄而不取消）。
    ///   **不能**靠 drop 实现：compio 的 `JoinHandle` 在 drop 时会
    ///   `cancel(true)` 取消任务。
    ///
    /// # 已知限制
    ///
    /// smol 后端的 `spawn_local` 任务：其本地执行器随句柄存活（句柄 poll 时
    /// 驱动执行器），detach 消费句柄后执行器被销毁，本地任务无法继续推进
    /// （等同取消）。因此 smol 的 `detach` 只对 `spawn`（全局执行器）任务有
    /// 完整语义。
    fn detach(self);
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
    F: Future,
{
    /// 阻塞当前线程，等待 `f` 完成并返回其结果。
    fn block_on(f: F) -> F::Output;
}

/// 暂停当前执行上下文一段时间。
pub trait TrDelay {
    /// 返回一个等待 `duration` 之后完成的 future。
    fn delay(duration: core::time::Duration) -> impl Future<Output = ()>;
}
