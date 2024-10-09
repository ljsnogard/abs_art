use core::future::Future;
use crate::task::JoinHandle;

pub fn spawn<F, T>(future: F) -> JoinHandle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    spawn_impl_::spawn(future)
}

pub fn spawn_local<F, T>(future: F) -> JoinHandle<T>
where
    F: Future<Output = T> + 'static,
    T: 'static,
{
    spawn_impl_::spawn_local(future)
}

pub fn spawn_blocking<F, T>(future: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    spawn_impl_::spawn_blocking(future)
}

#[cfg(feature = "runtime-async-std")]
mod spawn_impl_ {
    use core::future::Future;
    use async_std::task;
    use crate::task::JoinHandle;

    pub fn spawn<F, T>(future: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        JoinHandle::from(task::spawn(future))
    }

    pub fn spawn_local<F, T>(future: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        JoinHandle::from(task::spawn_local(future))
    }

    pub fn spawn_blocking<F, T>(future: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        JoinHandle::from(task::spawn_blocking(future))
    }
}

#[cfg(feature = "runtime-smol")]
mod spawn_impl_ {
    use core::future::Future;
    use crate::task::JoinHandle;

    use async_executor::LocalExecutor;
    use scoped_tls::scoped_thread_local;

    scoped_thread_local!(static LOCAL_EX: LocalExecutor);

    pub fn spawn<F, T>(future: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        JoinHandle::from(smol::spawn(future))
    }

    pub fn spawn_local<F, T>(future: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        if LOCAL_EX.is_set() {
            let task = LOCAL_EX.with(|local_ex| local_ex.spawn(future));
            JoinHandle::from(task)
        } else {
            panic!("`spawn_local()` must be called from a `LocalExecutor`")
        }
    }

    pub fn spawn_blocking<F, T>(future: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        // JoinHandle::from(task::spawn_blocking(future))
        let _ = future;
        unimplemented!()
    }
}

#[cfg(feature = "runtime-tokio")]
mod spawn_impl_ {
    use core::future::Future;
    use tokio::task;
    use crate::task::JoinHandle;

    pub fn spawn<F, T>(future: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        JoinHandle::from(task::spawn(future))
    }

    pub fn spawn_local<F, T>(future: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        JoinHandle::from(task::spawn_local(future))
    }

    pub fn spawn_blocking<F, T>(future: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        JoinHandle::from(task::spawn_blocking(future))
    }
}

#[cfg(not(any(
    feature = "runtime-async-std",
    feature = "runtime-tokio",
    feature = "runtime-smol"
)))]
mod spawn_impl_ {
    use core::future::Future;
    use crate::task::JoinHandle;

    pub fn spawn<F, T>(_: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        unimplemented!()
    }

    pub fn spawn_local<F, T>(_: F) -> JoinHandle<T>
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        unimplemented!()
    }

    pub fn spawn_blocking<F, T>(_: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        unimplemented!()
    }
}
