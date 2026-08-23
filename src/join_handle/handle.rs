use core::{
    fmt,
    future::IntoFuture,
    marker::PhantomPinned,
    pin::Pin,
    task::{Context, Poll},
};

use super::runtime_spec_;

type RtHandleWrapper<T> = runtime_spec_::RuntimeHandleWrapper<T>;
type MadeJoinFuture<'a, T> = <() as TrFutureFactory<'a, T>>::MadeFuture;

pub struct JoinHandle<T>
where
    T: 'static,
{
    handle_: Option<RtHandleWrapper<T>>,
    future_: Option<MadeJoinFuture<'static, T>>,
    _pinned_: PhantomPinned,
}

impl<T> JoinHandle<T> {
    pub(crate) fn from_wrapper(wrapper: RtHandleWrapper<T>) -> Self {
        JoinHandle {
            handle_: Option::Some(wrapper),
            future_: Option::None,
            _pinned_: PhantomPinned,
        }
    }

    async fn join_async_(&mut self) -> Result<T, JoinError> {
        let Option::Some(handle_impl) = self.handle_.take() else {
            unreachable!()
        };
        handle_impl
            .into_future()
            .await
            .map_err(self::JoinError)
    }
}

impl<T> From<runtime_spec_::RuntimeJoinHandle<T>> for JoinHandle<T> {
    #[inline]
    fn from(handle: runtime_spec_::RuntimeJoinHandle<T>) -> Self {
        let wrapper = runtime_spec_::RuntimeHandleWrapper::from_runtime_handle(handle);
        JoinHandle::from_wrapper(wrapper)
    }
}

impl<T> From<runtime_spec_::RuntimeHandleWrapper<T>> for JoinHandle<T> {
    #[inline]
    fn from(wrapper: runtime_spec_::RuntimeHandleWrapper<T>) -> Self {
        JoinHandle::from_wrapper(wrapper)
    }
}

/// Wrapper for runtime specific JoinError
pub struct JoinError(runtime_spec_::RuntimeJoinError);

impl JoinError {
    pub fn into_inner(self) -> runtime_spec_::RuntimeJoinError {
        self.0
    }
}

impl core::convert::AsRef<runtime_spec_::RuntimeJoinError> for JoinError {
    fn as_ref(&self) -> &runtime_spec_::RuntimeJoinError {
        &self.0
    }
}

impl fmt::Debug for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl core::error::Error for JoinError {}

impl<T> Future for JoinHandle<T>
where
    T: 'static,
{
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this: core::ptr::NonNull<Self> = unsafe {
            let p = self.get_unchecked_mut();
            core::ptr::NonNull::new_unchecked(p)
        };
        loop {
            let mut fut_field_ptr = unsafe {
                let ptr = &mut this.as_mut().future_;
                ::core::ptr::NonNull::new_unchecked(ptr)
            };
            let opt_fut = unsafe { fut_field_ptr.as_mut() };
            if let Option::Some(fut) = opt_fut {
                let fut_pin = unsafe { ::core::pin::Pin::new_unchecked(fut) };
                break fut_pin.poll(cx)
            } else {
                let p = unsafe { ::core::pin::Pin::new_unchecked(this.as_mut()) };
                let fut = <() as TrFutureFactory<T>>::make_future(p);
                let fut_field_mut = unsafe { fut_field_ptr.as_mut() };
                *fut_field_mut = Option::Some(fut);
            }
        }
    }
}

trait TrFutureFactory<'a, T> {
    type MadeFuture: Future;

    fn make_future(f: Pin<&'a mut JoinHandle<T>>) -> Self::MadeFuture;
}

impl<'a, T> TrFutureFactory<'a, T> for () {
    type MadeFuture = impl Future<Output = Result<T, JoinError>>;

    fn make_future(f: Pin<&'a mut JoinHandle<T>>) -> Self::MadeFuture {
        let join_handle = unsafe { f.get_unchecked_mut() };
        join_handle.join_async_()
    }
}
