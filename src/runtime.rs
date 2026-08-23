#[cfg(feature = "join_handle")]
use crate::join_handle::JoinHandle;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Runtime {
    Compio,
    Smol,
    Tokio,
}

impl Runtime {
    #[cfg(feature = "runtime-compio")]
    pub const fn current() -> Self {
        Runtime::Compio
    }

    #[cfg(feature = "runtime-smol")]
    pub const fn current() -> Self {
        Runtime::Smol
    }

    #[cfg(feature = "runtime-tokio")]
    pub const fn current() -> Self {
        Runtime::Tokio
    }
}

impl Runtime {
    /// 阻塞当前线程，等待 `future` 完成，同时不影响 tokio 运行时的调度。
    ///
    /// 必须在 tokio 运行时上下文内调用（例如在 `Runtime::block_on` 或某个
    /// 由 `tokio::spawn` 创建的任务内部）；否则 `Handle::current()` 会 panic。
    #[cfg(feature = "block_on")]
    pub fn block_on<F>(future: F) -> F::Output
    where
        Self: TrBlockOn<F>,
        F: Future + 'static,
    {
        <Runtime as TrBlockOn<F>>::block_on(future)
    }

    #[cfg(feature = "spawn_send")]
    pub fn spawn<F>(future: F) -> JoinHandle<<F as Future>::Output>
    where
        Self: TrSpawnSend<F>,
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        <Runtime as TrSpawnSend<F>>::spawn(future)
    }

    #[cfg(feature = "spawn_local")]
    pub fn spawn_local<F>(future: F) -> JoinHandle<<F as Future>::Output>
    where
        Self: TrSpawnLocal<F>,
        F: Future + 'static,
        <F as Future>::Output: 'static,
    {
        <Runtime as TrSpawnLocal<F>>::spawn_local(future)
    }

    #[cfg(feature = "spawn_blocking")]
    pub fn spawn_blocking<F, T>(f: F) -> JoinHandle<T>
    where
        Self: TrSpawnBlocking<F, T>,
        F: FnOnce() -> T,
        T: Send + 'static,
    {
        <Runtime as TrSpawnBlocking<F, T>>::spawn_blocking(f)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime::current()
    }
}

/// The runtime can spawn a task into a global work stealing queue.
#[cfg(feature = "spawn_send")]
pub trait TrSpawnSend<F>
where
    F: Future + Send + 'static,
    <F as Future>::Output: Send + 'static,
{
    fn spawn(future: F) -> JoinHandle<F::Output>;
}

/// The runtime can spawn a task for thread-local only work queue.
#[cfg(feature = "spawn_local")]
pub trait TrSpawnLocal<F>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
{
    fn spawn_local(future: F) -> JoinHandle<F::Output>;
}

#[cfg(feature = "spawn_blocking")]
pub trait TrSpawnBlocking<F, T>
where
    F: FnOnce() -> T,
    T: Send + 'static,
{
    fn spawn_blocking(f: F) -> JoinHandle<T>;
}

/// Tell the async runtime, this thread should wait on async task running, then
/// await the result without affecting the schedule of async runtime.
#[cfg(feature = "block_on")]
pub trait TrBlockOn<F>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
{
    fn block_on(f: F) -> F::Output;
}
