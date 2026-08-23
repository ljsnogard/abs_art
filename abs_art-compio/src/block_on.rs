//! `block_on`：阻塞当前线程等待 future 完成，同时不影响 compio 运行时的调度。

use core::future::Future;

use compio::runtime::Runtime as CompioRuntime;

use crate::Runtime;
use abs_art::{FULL, HasBlockOn, TrBlockOn};

impl Runtime<FULL> {
    /// 阻塞当前线程，等待 `future` 完成，同时不影响 compio 运行时的调度。
    ///
    /// 必须在 compio 运行时上下文内调用（例如在 `Runtime::block_on` 或某个
    /// 由 `Runtime::spawn` 创建的任务内部）；否则 `Runtime::with_current`
    /// 会 panic。
    pub fn block_on<F>(future: F) -> F::Output
    where
        Self: TrBlockOn<F>,
        F: Future + 'static,
    {
        <Self as TrBlockOn<F>>::block_on(future)
    }
}

impl<F, const CAPS: usize> TrBlockOn<F> for Runtime<CAPS>
where
    F: Future + 'static,
    <F as Future>::Output: 'static,
    [(); CAPS]: HasBlockOn,
{
    /// 通过 `CompioRuntime::with_current` 获取当前线程的环境运行时，再调用其
    /// `block_on` 阻塞驱动 `future`。
    ///
    /// compio 的 `block_on` 在等待期间会循环执行「轮询 future → 驱动 executor
    /// → 轮询驱动」：因此 `future` 处于 pending 时，同运行时内 `spawn` 的其他
    /// 任务仍会被推进，即「不影响运行时调度」。
    fn block_on(future: F) -> F::Output {
        CompioRuntime::with_current(|rt| {
            rt.block_on(future)
        })
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `Runtime::block_on` 单元测试。
    //!
    //! 测试全部在真实的 compio 运行时上执行，验证 `Runtime::block_on` 的
    //! 返回值、对运行时任务的驱动以及「必须处于 compio 运行时上下文内」的
    //! 使用契约。

    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use compio::runtime::Runtime as CompioRuntime;

    use crate::Runtime;

    /// 目的：验证在 compio 运行时内部调用 `Runtime::block_on` 能正确返回 future
    /// 的输出，并且支持「嵌套」使用——外层 `rt.block_on` 已经进入运行时上下文，
    /// 内层 `Runtime::block_on` 通过 `with_current` 拿到同一个运行时再阻塞驱动
    /// future。
    ///
    /// 实施策略：创建 compio 运行时，在最外层 `rt.block_on` 中调用
    /// `Runtime::block_on` 去驱动一个返回常量表达式的 future，并把结果带出外层。
    ///
    /// 通过依据：外层 `rt.block_on` 的返回值等于 6 * 7 == 42，且整个过程没有
    /// panic（嵌套的 `with_current` 与 `enter` 均正常）。
    #[test]
    fn block_on_inside_runtime_returns_output() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async { Runtime::block_on(async { 6 * 7 }) });

        assert_eq!(out, 42);
    }

    /// 目的：验证 `Runtime::block_on` 阻塞等待期间，compio 运行时中由 `rt.spawn`
    /// 产生的任务仍会被调度执行——即「不影响运行时调度」的契约。
    ///
    /// 实施策略：先 `rt.spawn` 一个后台任务（循环递增原子计数器后返回计数值），
    /// 然后在外层 `rt.block_on` 中调用 `Runtime::block_on` 去 await 该任务的
    /// JoinHandle。compio 的 JoinHandle 自身不会内联执行任务，它只是注册 waker
    /// 并等待 executor 把任务跑完；因此若实现没有在等待期间驱动 executor，这个
    /// await 将永远无法完成。
    ///
    /// 通过依据：整个场景放在独立线程中执行，并用 `recv_timeout` 限制等待时间。
    /// 若 10 秒内 `Runtime::block_on` 返回、JoinHandle 结果为 `Ok` 且计数器与
    /// 返回值都等于 1000，说明后台任务确实在 block_on 期间被驱动执行了，测试
    /// 通过；若超时、线程 panic 或数值不符，则测试失败。
    #[test]
    fn block_on_drives_spawned_tasks() {
        const TARGET: usize = 1000;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let rt = CompioRuntime::new().unwrap();

            let result = rt.block_on(async {
                let counter = std::sync::Arc::new(AtomicUsize::new(0));
                let c = counter.clone();

                let handle = rt.spawn(async move {
                    for _ in 0..TARGET {
                        c.fetch_add(1, Ordering::Relaxed);
                    }
                    c.load(Ordering::Relaxed)
                });

                let value = Runtime::block_on(async { handle.await.unwrap() });
                assert_eq!(counter.load(Ordering::Relaxed), TARGET);
                value
            });

            let _ = tx.send(result);
        });

        let result = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("测试失败：等待结果超时或线程 panic（{e:?}）"));
        assert_eq!(result, TARGET);
    }

    /// 目的：验证 `Runtime::block_on` 返回的 future 输出在多次调用之间相互独立，
    /// 不会因为前一次调用而残留任何运行时状态（每次调用都通过 `with_current`
    /// 重新获取环境运行时）。
    ///
    /// 实施策略：在同一个外层 `rt.block_on` 上下文中先后调用两次
    /// `Runtime::block_on`，每次驱动不同的 future，并把两次结果相加。
    ///
    /// 通过依据：外层 `rt.block_on` 返回 1 + 2 + 3 + 4 == 10，说明两次调用都
    /// 正常工作且互不干扰。
    #[test]
    fn block_on_multiple_calls_are_independent() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let a = Runtime::block_on(async { 1 + 2 });
            let b = Runtime::block_on(async { 3 + 4 });
            a + b
        });

        assert_eq!(out, 10);
    }

    /// 目的：验证在没有任何 compio 运行时上下文的线程中调用 `Runtime::block_on`
    /// 会 panic——compio 实现依赖 `Runtime::with_current` 获取环境运行时，而该
    /// 函数在没有环境运行时时必然 panic。这固定了「必须处于 compio 运行时上下文
    /// 内」的使用契约。
    ///
    /// 实施策略：不创建也不进入任何 compio 运行时，直接在测试线程中调用
    /// `Runtime::block_on`。
    ///
    /// 通过依据：测试按预期捕获 panic（`expected = "not in a compio runtime"`
    /// 匹配 `with_current` 的 panic 信息）即为通过；若没有 panic，则测试失败，
    /// 说明实现与契约不符。
    #[test]
    #[should_panic(expected = "not in a compio runtime")]
    fn block_on_outside_runtime_panics() {
        Runtime::block_on(async { 42 });
    }

    /// 目的：验证 Tag 模式下，声明了 `BLOCK_ON` 能力的 `Runtime<Caps>` 确实
    /// 实现了 `TrBlockOn`（编译期能力检查的正向用例）。
    ///
    /// 实施策略：在 compio 运行时上下文内，用
    /// `Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>` 的 trait 关联函数调用
    /// `block_on` 驱动一个 future。
    ///
    /// 通过依据：返回值为 40 + 2 == 42；若 `HasBlockOn` 标记或条件化 trait
    /// impl 有误，将无法编译。
    #[test]
    fn tagged_runtime_implements_block_on() {
        use abs_art::{BLOCK_ON, SPAWN_LOCAL, TrBlockOn};

        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            <Runtime<{ BLOCK_ON | SPAWN_LOCAL }> as TrBlockOn<_>>::block_on(async {
                40 + 2
            })
        });
        assert_eq!(out, 42);
    }
}
