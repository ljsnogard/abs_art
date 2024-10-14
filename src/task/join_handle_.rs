use core::{
    error, fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use pin_project::pin_project;
use pin_utils::pin_mut;

#[pin_project]
pub struct JoinHandle<T>(#[pin]join_impl_::JoinHandleImpl<T>);

impl<T> JoinHandle<T> {
    async fn join_async_(self: Pin<&mut Self>) -> Result<T, JoinError> {
        self.project().0.await.map_err(JoinError)
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let f = self.join_async_();
        pin_mut!(f);
        f.poll(cx)
    }
}

#[cfg(feature = "runtime-async-std")]
impl<T> From<async_std::task::JoinHandle<T>> for JoinHandle<T> {
    fn from(handle: async_std::task::JoinHandle<T>) -> Self {
        JoinHandle(join_impl_::JoinHandleImpl::from_handle(handle))
    }
}

#[cfg(feature = "runtime-smol")]
impl<T> From<smol::Task<T>> for JoinHandle<T> {
    fn from(handle: smol::Task<T>) -> Self {
        JoinHandle(join_impl_::JoinHandleImpl::from_handle(handle))
    }
}

#[cfg(feature = "runtime-tokio")]
impl<T> From<tokio::task::JoinHandle<T>> for JoinHandle<T> {
    fn from(handle: tokio::task::JoinHandle<T>) -> Self {
        JoinHandle(join_impl_::JoinHandleImpl::from_handle(handle))
    }
}

pub struct JoinError(join_impl_::JoinErrorImpl);

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

impl error::Error for JoinError {}

#[cfg(feature = "runtime-async-std")]
mod join_impl_ {
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
    pub(super) struct JoinHandleImpl<T> {
        #[pin]handle_: async_std::task::JoinHandle<T>,
    }

    impl<T> JoinHandleImpl<T> {
        pub fn from_handle(handle: async_std::task::JoinHandle<T>) -> Self {
            JoinHandleImpl { handle_: handle }
        }

        async fn join_async_(self: Pin<&mut Self>) -> Result<T, JoinErrorImpl> {
            let t = self.project().handle_.await;
            Result::Ok(t)
        }
    }

    impl<T> Future for JoinHandleImpl<T> {
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

    pub(super) struct JoinErrorImpl(Infallible);

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

#[cfg(feature = "runtime-smol")]
mod join_impl_ {
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
    pub(super) struct JoinHandleImpl<T> {
        #[pin]handle_: smol::Task<T>,
    }

    impl<T> JoinHandleImpl<T> {
        pub fn from_handle(handle: smol::Task<T>) -> JoinHandleImpl<T> {
            JoinHandleImpl { handle_: handle }
        }

        async fn join_async_(self: Pin<&mut Self>) -> Result<T, JoinErrorImpl> {
            let t = self.project().handle_.await;
            Result::Ok(t)
        }
    }

    impl<T> Future for JoinHandleImpl<T> {
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

    pub(super) struct JoinErrorImpl(Infallible);

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

#[cfg(feature = "runtime-tokio")]
mod join_impl_ {
    use core::{
        fmt,
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };
    use pin_project::pin_project;
    use pin_utils::pin_mut;

    #[pin_project]
    pub(super) struct JoinHandleImpl<T> {
        #[pin]handle_: tokio::task::JoinHandle<T>,
    }

    impl<T> JoinHandleImpl<T> {
        pub fn from_handle(handle: tokio::task::JoinHandle<T>) -> JoinHandleImpl<T> {
            JoinHandleImpl { handle_: handle }
        }

        async fn join_async_(self: Pin<&mut Self>) -> Result<T, JoinErrorImpl> {
            self.project()
                .handle_
                .await
                .map_err(JoinErrorImpl)
        }
    }

    impl<T> Future for JoinHandleImpl<T> {
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

    pub(super) struct JoinErrorImpl(tokio::task::JoinError);

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

#[cfg(not(any(
    feature = "runtime-async-std",
    feature = "runtime-tokio",
    feature = "runtime-smol",
)))]
mod join_impl_ {
    use core::{
        future::Future,
        marker::PhantomData,
        pin::Pin,
        task::{Context, Poll},
    };
    use std::marker::PhantomData;

    pub(super) struct JoinHandleImpl<T>(PhantomData<T>);

    impl<T> JoinHandleImpl<T> {
        async fn join_async_(self: Pin<&mut Self>) -> Result<T, JoinErrorImpl> {
            unimplemented!()
        }
    }

    impl<T> Future for JoinHandleImpl<T> {
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

    pub(super) struct JoinErrorImpl(PhantomData<()>);

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
