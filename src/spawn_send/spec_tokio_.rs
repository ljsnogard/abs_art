use crate::{
    join_handle::{JoinHandle, JoinError},
    runtime::{Runtime, TrSpawnSend},
};

impl Runtime {
    /// Spawn a task by `tokio::task::spawn`
    /// 
    /// ```
    /// let runtime = Runtime::current();
    /// runtime.spawn(async { 42 });
    /// ```
    pub fn spawn<F>(future: F) -> F::Output
    where
        Self: TrSpawnSend<F>,
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        <Runtime as TrSpawnSend<F>>::spawn_send(future)
    }
}

impl<F> TrSpawnSend<F> for Runtime
where
    F: Future + Send + 'static,
    <F as Future>::Output: Send + 'static,
{
    fn spawn(future: F) -> JoinHandle<<F as Future>::Output> {
        tokio::task::spawn(future).into()
    }
}
