//! 针对 compio 后端的 `Runtime::block_on` 单元测试。
//!
//! 该文件只在启用 `runtime-compio` feature 且处于测试构建（`cfg(test)`）时
//! 编译，不会影响正常的库构建。测试全部在真实的 compio 运行时上执行，验证
//! `abs_art::Runtime::block_on` 的返回值、对运行时任务的驱动以及「必须处于
//! compio 运行时上下文内」的使用契约。

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use compio::runtime::Runtime as CompioRuntime;

use crate::runtime::Runtime;

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
