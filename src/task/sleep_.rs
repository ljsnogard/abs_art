use core::time::Duration;
use super::{spawn, spawn_local, JoinHandle};

pub async fn sleep(duration: Duration) {
    sleep_impl_::sleep(duration).await
}

pub fn delayed<X, F>(
    interval: Duration,
    f: F,
) -> JoinHandle<X>
where
    X: Send + 'static,
    F: FnOnce() -> X + 'static + Send,
{
    spawn(exec_delayed_(interval, f))
}

pub fn delayed_local<X, F>(
    interval: Duration,
    f: F,
) -> JoinHandle<X>
where
    X: 'static,
    F: 'static + FnOnce() -> X,
{
    async fn exec_delayed_<X>(
        duration: Duration,
        execute: impl FnOnce() -> X,
    ) -> X {
        sleep(duration).await;
        execute()
    }
    spawn_local(exec_delayed_(interval, f))
}

async fn exec_delayed_<X>(
    duration: Duration,
    execute: impl FnOnce() -> X,
) -> X {
    sleep(duration).await;
    execute()
}

#[cfg(feature = "runtime-async-std")]
mod sleep_impl_ {
    pub(super) use async_std::task::sleep;
}

#[cfg(feature = "runtime-smol")]
mod sleep_impl_ {
    use core::time::Duration;
    use smol::Timer;

    pub(super) async fn sleep(duration: Duration) {
        Timer::after(duration).await;
    }
}

#[cfg(feature = "runtime-tokio")]
mod sleep_impl_ {
    pub(super) use tokio::time::sleep;
}

#[cfg(not(any(
    feature = "runtime-async-std",
    feature = "runtime-tokio",
    feature = "runtime-smol"
)))]
mod sleep_impl_ {
    pub(super) async fn sleep(_: core::time::Duration) {
        unimplemented!()
    }
}
