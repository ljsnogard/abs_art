use core::future::IntoFuture;

pub(crate) type RuntimeJoinHandle<T> = tokio::task::JoinHandle<T>;
pub(crate) type RuntimeJoinError = tokio::task::JoinError;

/// Wrapper for `tokio::task::JoinHandle<T>`
pub(crate) struct RuntimeHandleWrapper<T>(RuntimeJoinHandle<T>);

impl<T> RuntimeHandleWrapper<T> {
    pub const fn from_runtime_handle(handle: RuntimeJoinHandle<T>) -> Self {
        RuntimeHandleWrapper(handle)
    }
}

impl<T> IntoFuture for RuntimeHandleWrapper<T> {
    type Output = Result<T, RuntimeJoinError>;
    type IntoFuture = RuntimeJoinHandle<T>;

    fn into_future(self) -> Self::IntoFuture {
        self.0
    }
}
