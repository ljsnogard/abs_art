//! `block_on`：阻塞当前线程等待 future 完成，同时不影响 tokio 运行时的调度。

use core::future::Future;

use tokio::runtime::Handle;

use crate::Runtime;
use abs_art::{FULL, HasBlockOn, TrBlockOn};

impl Runtime<FULL> {
    /// 阻塞当前线程，等待 `future` 完成，同时不影响 tokio 运行时的调度。
    ///
    /// 必须在 tokio 运行时上下文内调用（例如在 `Runtime::block_on` 或某个
    /// 由 `tokio::spawn` 创建的任务内部）；否则 `Handle::current()` 会 panic。
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
    /// 先通过 `tokio::task::block_in_place` 把当前 worker 线程（及其任务队列）
    /// 让渡回阻塞线程池，让其他任务可以继续在该线程上被调度；再在闭包内用
    /// `Handle::current().block_on(future)` 驱动 `future` 直到完成。
    ///
    /// 这是 tokio 官方文档推荐的「在多线程运行时内同步等待 async 结果」的模式：
    /// 当前线程被阻塞的同时，运行时的其他任务仍能得到调度，即「不影响运行时调度」。
    ///
    /// 注意：`block_in_place` 不允许在 current_thread 运行时内使用（没有其他
    /// worker 线程可以承接任务），此时会 panic。
    fn block_on(future: F) -> <F as Future>::Output {
        tokio::task::block_in_place(move || {
            // 在闭包内，通过当前运行时句柄的 block_on 来等待
            Handle::current().block_on(future)
        })
    }
}

#[cfg(test)]
mod tests {
    //! 针对 tokio 后端的 `Runtime::block_on` 单元测试。
    //!
    //! 测试全部在真实的多线程 tokio 运行时上执行，验证 `Runtime::block_on`
    //! 的返回值、使用场景与「不影响运行时调度」这一契约。

    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use abs_art::{BLOCK_ON, SPAWN_LOCAL, TrBlockOn};

    use crate::Runtime;

    /// 目的：验证在 tokio 多线程运行时内部调用 `Runtime::block_on` 能正确返回
    /// future 的最终输出。
    ///
    /// 实施策略：创建一个多线程 tokio 运行时，在最外层的 `rt.block_on` 上下文中
    /// （此时当前线程处于「已进入运行时」状态，允许使用 `block_in_place`）调用
    /// `Runtime::block_on` 去驱动一个返回常量表达式的 future，并把结果带出外层
    /// `rt.block_on`。
    ///
    /// 通过依据：外层 `rt.block_on` 的返回值等于 future 的计算结果（1 + 2 == 3），
    /// 且整个过程没有 panic。
    #[test]
    fn block_on_inside_runtime_returns_output() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();

        let out = rt.block_on(async { Runtime::block_on(async { 1 + 2 }) });

        assert_eq!(out, 3);
    }

    /// 目的：验证 `Runtime::block_on` 也能在运行时的 worker 线程（即由
    /// `tokio::spawn` 创建的任务内部）正常使用，而不只是在外层 `rt.block_on`
    /// 的上下文里可用。
    ///
    /// 实施策略：先 `tokio::spawn` 一个任务，该任务内部调用 `Runtime::block_on`
    /// （此时当前线程是运行时的 worker 线程），随后在外层 `rt.block_on` 中 await
    /// 该任务的 JoinHandle 取回结果。
    ///
    /// 通过依据：JoinHandle 的 await 结果成功（`Ok`）且等于 40 + 2 == 42；
    /// 同时证明在 worker 线程内调用不会 panic。
    #[test]
    fn block_on_inside_spawned_task_returns_output() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();

        let task = rt.spawn(async { Runtime::block_on(async { 40 + 2 }) });
        let out = rt.block_on(async { task.await.unwrap() });

        assert_eq!(out, 42);
    }

    /// 目的：验证 `Runtime::block_on` 阻塞当前线程期间，运行时的其他任务仍然能
    /// 被调度执行——即 trait 文档中「不影响异步运行时的调度」这一核心契约。
    ///
    /// 实施策略：创建一个只有 1 个 worker 线程的多线程运行时，让当前线程通过
    /// `rt.block_on` 充当该 worker；先 `tokio::spawn` 一个后台任务，它在循环中
    /// 递增原子计数器并 `yield_now`；随后调用 `Runtime::block_on` 阻塞等待计数器
    /// 达到目标值。若实现没有通过 `block_in_place` 把 worker 让渡出去，后台任务
    /// 将永远得不到调度，等待循环将无法退出（死锁）。
    ///
    /// 通过依据：整个场景被放到一个独立线程中执行，并用 `recv_timeout` 限制等待
    /// 时间——若 10 秒内收到结果且最终计数等于目标值（10_000），说明后台任务在
    /// block_on 阻塞期间确实被调度执行了，测试通过；若超时、线程 panic 或计数
    /// 不足，则测试失败。
    #[test]
    fn block_on_keeps_runtime_scheduling() {
        const TARGET: usize = 10_000;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .build()
                .unwrap();

            let count = rt.block_on(async {
                let counter = std::sync::Arc::new(AtomicUsize::new(0));
                let c = counter.clone();

                let bg = tokio::spawn(async move {
                    for _ in 0..TARGET {
                        c.fetch_add(1, Ordering::Relaxed);
                        tokio::task::yield_now().await;
                    }
                });

                let wait_counter = counter.clone();
                Runtime::block_on(async move {
                    while wait_counter.load(Ordering::Relaxed) < TARGET {
                        tokio::task::yield_now().await;
                    }
                });

                bg.await.unwrap();
                counter.load(Ordering::Relaxed)
            });

            let _ = tx.send(count);
        });

        let count = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("测试失败：等待结果超时或线程 panic（{e:?}）"));
        assert_eq!(count, TARGET);
    }

    /// 目的：验证在没有任何 tokio 运行时上下文的线程中调用 `Runtime::block_on`
    /// 会 panic——tokio 实现依赖 `Handle::current()` 获取环境运行时，而该函数在
    /// 没有环境运行时的情况下必然 panic。这固定了「必须处于运行时上下文内」的
    /// 使用契约。
    ///
    /// 实施策略：不创建也不进入任何 tokio 运行时，直接在测试线程中调用
    /// `Runtime::block_on`。
    ///
    /// 通过依据：测试按预期捕获 panic（`expected = "no reactor running"` 匹配
    /// `Handle::current()` 的 panic 信息）即为通过；若没有 panic（即实现静默地
    /// 创建了新的运行时或返回了结果），则测试失败，说明实现与契约不符。
    #[test]
    #[should_panic(expected = "no reactor running")]
    fn block_on_outside_runtime_panics() {
        Runtime::block_on(async { 42 });
    }

    /// 目的：验证 Tag 模式下，声明了 `BLOCK_ON` 能力的 `Runtime<Caps>` 确实
    /// 实现了 `TrBlockOn`（编译期能力检查的正向用例）。
    ///
    /// 实施策略：用 `Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>` 调用 `current()`
    /// 取得标记值，再通过 trait 关联函数调用 `block_on` 驱动一个 future。
    ///
    /// 通过依据：返回值为 40 + 2 == 42；若 `HasBlockOn` 标记或条件化 trait
    /// impl 有误，将无法编译。
    #[test]
    fn tagged_runtime_implements_block_on() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();

        let out = rt.block_on(async {
            <Runtime<{ BLOCK_ON | SPAWN_LOCAL }> as TrBlockOn<_>>::block_on(async {
                40 + 2
            })
        });
        assert_eq!(out, 42);
    }
}
