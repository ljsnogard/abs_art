

#[cfg(feature = "runtime_compio")]
impl<T> From<compio::runtime::JoinHandle<T>> for JoinHandle<T> {
    #[inline]
    fn from(handle: compio::runtime::JoinHandle<T>) -> Self {
        JoinHandle::from_wrapper(RtHandleWrapper::<T>::from_runtime_handle(handle))
    }
}

#[cfg(feature = "runtime_smol")]
impl<T> From<smol::Task<T>> for JoinHandle<T> {
    fn from(handle: smol::Task<T>) -> Self {
        JoinHandle::from_wrapper(RtHandleWrapper::<T>::from_runtime_handle(handle))
    }
}

#[cfg(feature = "runtime_tokio")]
impl<T> From<tokio::task::JoinHandle<T>> for JoinHandle<T> {
    fn from(handle: tokio::task::JoinHandle<T>) -> Self {
        JoinHandle::from_wrapper(RtHandleWrapper::<T>::from_runtime_handle(handle))
    }
}


#[cfg(feature = "runtime_compio")]
mod runtime_spec_impl_ {

}

#[cfg(feature = "runtime_smol")]
mod runtime_spec_impl_ {
    use core::{
        convert::Infallible,
        fmt,
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };
    use pin_project::pin_project;
    use pin_utils::pin_mut;

    #[pin_project]
    pub(crate) struct JoinHandleImpl<T> {
        #[pin]handle_: smol::Task<T>,
    }

    impl<T> RtHandleWrapper<T> {
        pub fn from_handle(handle: smol::Task<T>) -> RtHandleWrapper<T> {
            RtHandleWrapper { handle_: handle }
        }

        async fn join_async_(self: Pin<&mut Self>) -> Result<T, JoinErrorImpl> {
            let t = self.project().handle_.await;
            Result::Ok(t)
        }
    }

    impl<T> Future for RtHandleWrapper<T> {
        type Output = Result<T, JoinErrorImpl>;

        fn poll(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Self::Output> {
            let f = self.join_async_();
            pin_mut!(f);
            f.poll(cx)
        }
    }

    pub(crate) struct JoinErrorImpl(Infallible);

    impl fmt::Debug for JoinErrorImpl {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl fmt::Display for JoinErrorImpl {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }
}

#[cfg(feature = "runtime_tokio")]
mod join_impl_ {

}
