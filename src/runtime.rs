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
