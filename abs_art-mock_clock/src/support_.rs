//! 测试替身：一个最小的「运行时 + join handle」，供本 crate 的单元测试使用。

use core::{
    cell::Cell,
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use abs_art::{RuntimeTag, TrAsyncRuntime, TrJoinHandle};

/// 假 join 错误：只需要实现 `core::error::Error`。
#[derive(Debug)]
pub struct FakeJoinErr_;

impl fmt::Display for FakeJoinErr_ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("fake join error")
    }
}

impl core::error::Error for FakeJoinErr_ {}

/// 假 join handle：第一次 poll 就返回 `Ok(value)`。
pub struct FakeJoin_<T>(Cell<Option<T>>);

impl<T> Future for FakeJoin_<T> {
    type Output = Result<T, FakeJoinErr_>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.0.replace(None) {
            Some(value) => Poll::Ready(Ok(value)),
            None => panic!("FakeJoin_ 被重复 poll"),
        }
    }
}

impl<T: 'static> TrJoinHandle<T> for FakeJoin_<T> {
    type JoinErr = FakeJoinErr_;

    fn detach(self) {}
}

/// 假运行时：只有身份，没有调度。
pub struct FakeRt_;

impl TrAsyncRuntime for FakeRt_ {
    type JoinHandle<T>
        = FakeJoin_<T>
    where
        T: 'static;

    fn about(&self) -> RuntimeTag {
        RuntimeTag::Smol
    }
}
