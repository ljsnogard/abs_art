use core::{fmt, future::IntoFuture};

pub(crate) type RuntimeJoinHandle<T> = smol::Task<T>;
pub(crate) type RuntimeJoinError = DummySmolJoinError;

/// Wrapper for `smol::Task<T>`
pub(crate) struct RuntimeHandleWrapper<T>(RuntimeJoinHandle<T>);

impl<T> RuntimeHandleWrapper<T> {
    pub const fn from_runtime_handle(handle: RuntimeJoinHandle<T>) -> Self {
        RuntimeHandleWrapper(handle)
    }

    async fn wrap_into_future_(self) -> Result<T, RuntimeJoinError> {
        let t = self.0.await;
        Result::Ok(t)
    }
}

impl<T> IntoFuture for RuntimeHandleWrapper<T> {
    type Output = Result<T, RuntimeJoinError>;
    type IntoFuture = impl Future<Output = Result<T, RuntimeJoinError>>;

    fn into_future(self) -> Self::IntoFuture {
        RuntimeHandleWrapper::wrap_into_future_(self)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DummySmolJoinError();

impl core::fmt::Display for DummySmolJoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DummySmolJoinError")
    }
}
