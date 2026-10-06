//! `spawn_send`：把任务投递到 smol 的**进程级**全局工作窃取队列。
//!
//! # 这里有一条 smol 的固有限制：全局队列**无法被运行时值钉住**
//!
//! smol 2.0.2 的 `spawn` 形状是（源码事实，`smol/src/spawn.rs`）：
//!
//! ```text
//! pub fn spawn<T: Send + 'static>(future: impl Future<Output = T> + Send + 'static) -> Task<T> {
//!     static GLOBAL: OnceCell<Executor<'_>> = OnceCell::new();
//!     fn global() -> &'static Executor<'static> { GLOBAL.get_or_init_blocking(|| { …起后台线程… }) }
//!     global().spawn(future)
//! }
//! ```
//!
//! 即：全局执行器是**进程级单例**，懒初始化、由后台线程（数量取环境变量
//! `SMOL_THREADS`，缺省 1）永久驱动，`smol` 自己也注明「the executor is kept
//! around forever」。因此**不存在**一个可以交给运行时值的句柄/所有权：
//!
//! - 不能克隆它（`OnceCell` 只给 `&'static`）；
//! - 不能 drop 它（进程级 static）；
//! - 不能替换它（`get_or_init_blocking` 之后永远返回同一个）。
//!
//! 本 crate 的选择是**如实转发**并写清限制，而不是伪造「每值一份全局队列」的假象。
//! 可观测后果：
//!
//! 1. **值被 drop 不会停止已投递的任务**（任务归进程所有，见 `join_handle.rs`
//!    的同名测试）；
//! 2. **同一个进程里两个运行时值的 `spawn` 共享同一条队列**，值之间没有边界，
//!    值的数量也不改变全局并发度；
//! 3. **全局并发度由 `SMOL_THREADS` 决定**，是进程级配置而非本值的能力。
//!
//! 需要「队列随值走」的隔离语义时，用
//! [`TrLocalScope::spawn_local`](abs_art::TrLocalScope::spawn_local)——那才是本值
//! 真正拥有的队列（另一条 smol 侧的现实：smol 不提供 `spawn_local`，本地队列本来
//! 就得由本 crate 自建，于是它顺理成章地成为「值持有的那一半」）。
//!
//! 对比 `abs_art-tokio`：tokio 的 `Handle` 是一个可克隆、可持有的运行时把手，
//! 值能钉住它；smol 没有对应物，这是两者唯一的形状差异。

use core::future::Future;

use abs_art::{HasSpawnSend, TrSpawnSend};

use crate::{Runtime, join_handle::JoinHandle};

impl<const CAPS: usize> TrSpawnSend for Runtime<CAPS>
where
    [(); CAPS]: HasSpawnSend,
{
    type JoinHandle<T> = JoinHandle<T> where T: 'static;

    /// 把 `future` 投递到 smol 的**进程级**全局执行器，返回句柄。
    ///
    /// 说明：`&self` 在实现里**不被读取**——这条能力在 smol 上确实不归值所有（见
    /// 模块文档）。保留 `&self` 形状是为了让抽象层的调用点三后端一致（业务库只写
    /// `rt.spawn(..)`），也让将来 smol 若支持自建全局执行器时，替换点只在这一处。
    fn spawn<F>(&self, future: F) -> Self::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static,
    {
        smol::spawn(future).into()
    }
}

#[cfg(test)]
mod tests {
    //! 针对 smol 后端的 `spawn_send` 功能单元测试。

    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use abs_art::{FULL, TrSpawnSend};

    use crate::Runtime;

    /// 目的：验证运行时值的 `spawn` 能把 future 投递到 smol 全局执行器，并通过
    /// 返回的 [`JoinHandle`](crate::JoinHandle) 取回结果。
    ///
    /// 手段：构造 `Runtime<{ FULL }>` 值，调用其 `spawn`，在 `smol::block_on`
    /// 里 await 返回的句柄。
    ///
    /// 判定：句柄结果为 `Ok(6 * 7 == 42)`。
    #[test]
    fn spawn_returns_output() {
        let value = Runtime::<{ FULL }>::current();
        let out = smol::block_on(async {
            let handle = value.spawn(async { 6 * 7 });
            handle.await.unwrap()
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证「全局队列不归值所有」这条限制：**两个不同的运行时值**投递的任务
    /// 落在**同一条**进程级队列上，因此把其中一个值 drop 掉不影响另一个值投递的
    /// 任务，两条任务的执行都只由后台线程负责。
    ///
    /// 手段：构造两个独立的运行时值，各自 `spawn` 一个递增共享 `AtomicUsize` 的
    /// 任务，随即把**两个值都 drop**，再在 `smol::block_on` 里 await 两个句柄
    /// （句柄不持有队列，因此 await 只等任务本身）。
    ///
    /// 判定：两个句柄都交回 `Ok`，且计数器恰为 2。若实现把任务偷偷绑在值上
    /// （例如自建一个「属于值」的全局执行器并在 drop 时销毁），句柄会永远 pending
    /// 或被取消，本用例便不会返回（挂死）或断言失败。
    #[test]
    fn global_queue_is_shared_across_values_and_outlives_them() {
        let counter = Arc::new(AtomicUsize::new(0));

        let first = {
            let c = Arc::clone(&counter);
            let value = Runtime::<{ FULL }>::current();
            value.spawn(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        };
        let second = {
            let c = Arc::clone(&counter);
            let value = Runtime::<{ FULL }>::current();
            value.spawn(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        };
        // 两个运行时值都已在块作用域内 drop；任务仍应跑完。

        let (a, b) = smol::block_on(async { (first.await, second.await) });
        a.expect("第一个全局任务应当成功");
        b.expect("第二个全局任务应当成功");
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }
}
