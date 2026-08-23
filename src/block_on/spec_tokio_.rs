use tokio::runtime::Handle;

use crate::runtime::{Runtime, TrBlockOn};

impl Runtime {
    pub fn block_on<F>(future: F) -> F::Output
    where
        Self: TrBlockOn<F>,
        F: Future + 'static,
    {
        <Runtime as TrBlockOn<F>>::block_on(future)
    }
}

impl<F> TrBlockOn<F> for Runtime
where
    F: Future + 'static,
{
    /// Call `tokio::task::block_in_place` with the closure that
    /// `Handle::current().block_on(f)`
    fn block_on(future: F) -> <F as Future>::Output {
        tokio::task::block_in_place(move || {
            // 2. 在闭包内，通过当前运行时句柄的 block_on 来等待
            Handle::current().block_on(future)
        })
    }
}
