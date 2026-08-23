use core::{
    convert::Infallible,
    fmt,
    future::{self, IntoFuture},
    marker::PhantomData,
};

use super::priv_sealed_::TrJoinHandleProxy;

pub(crate) struct JoinHandleImpl<T> {
    _never_: Infallible,
    _use_t_: PhantomData<T>,
}

impl<T> IntoFuture for RtHandleWrapper<T> {
    type Output = Infallible;
    type IntoFuture = future::Pending<Infallible>;

    fn into_future(self) -> Self::IntoFuture {
        unreachable!()
    }
}

impl<T> TrJoinHandleProxy for RtHandleWrapper<T> {
    type TargetHandle = future::Pending<Infallible>;
    type JoinOutput = T;
    type TargetJoinErr = Infallible;
}

pub(crate) struct JoinErrorImpl(PhantomData<()>);

impl fmt::Debug for JoinErrorImpl {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        unreachable!()
    }
}

impl fmt::Display for JoinErrorImpl {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        unreachable!()
    }
}
