use compio::runtime::Runtime as CompioRuntime;

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
    /// Call
    fn block_on(future: F) -> F::Output {
        CompioRuntime::with_current(|rt| {
            rt.block_on(future)
        })
    }
}
