//! `block_on`：阻塞当前线程等待 future 完成，同时不影响 compio 运行时的调度。
//!
//! 阻塞驱动入口是**值方法**：driving 用的是 `self` 抓住的那份 compio 运行时，
//! 而不是「当前线程恰好进入了哪个运行时」。这是值化在 compio 上的直接收益
//! ——`Runtime::block_on` 因此可以**在上下文之外**调用（它自己 `enter`）。

use core::future::Future;

use abs_art::{HasBlockOn, TrBlockOn};

use crate::Runtime;

impl<const CAPS: usize> TrBlockOn for Runtime<CAPS>
where
    [(); CAPS]: HasBlockOn,
{
    /// 用**本值抓住的** compio 运行时阻塞驱动 `future`。
    ///
    /// compio 的 `Runtime::block_on` 内部先 `enter`（把这份运行时设为当前线程
    /// 上下文），再循环执行「轮询 future → tick executor → 轮询驱动」。于是：
    ///
    /// - 调用点**不必**已处于 compio 运行时上下文内——上下文由本值提供；
    /// - `future` 处于 pending 时，同一运行时上的其他任务（含经
    ///   [`TrLocalScope::spawn_local`](abs_art::TrLocalScope::spawn_local) 投递的本地
    ///   任务）仍会被推进，即「不影响运行时调度」。
    ///
    /// 本地作用域（`local_scope` feature）由同一次 `block_on` 一并驱动：compio 的执行器
    /// 队列本来就归运行时所有、由运行时自己 tick，而 [`LocalScope`](crate::LocalScope)
    /// 钉住的正是这份运行时，因此「阻塞等待」与「驱动本地队列」是同一件事。
    /// 需要显式指定驱动者时，也可以用 [`TrLocalScope::block_on`](abs_art::TrLocalScope::block_on)。
    ///
    /// # Panics
    ///
    /// 本方法自身不 panic。但若 `future` 内部依赖 compio 的**环境式**入口
    /// （例如 `TrDelay::delay`，它在线程本地注册计时器），注册点看到的就是本值
    /// `enter` 出来的上下文，因此仍然一致。
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future,
    {
        self.rt_.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    //! 针对 compio 后端的 `TrBlockOn::block_on` 单元测试。
    //!
    //! 测试全部在真实的 compio 运行时上执行，验证值方法的返回值、对同运行时
    //! 任务的驱动、以及「值自带上下文」这一新契约。

    use std::{cell::Cell, rc::Rc, sync::mpsc, thread, time::Duration};

    use abs_art::{BLOCK_ON, SPAWN_LOCAL, TrBlockOn, TrLocalScope};
    use compio::runtime::Runtime as CompioRuntime;

    use crate::Runtime;

    /// 目的：验证在 compio 运行时内部用**运行时值**调用 `block_on` 能正确返回
    /// future 的输出，并且支持「嵌套」使用——外层 `rt.block_on` 已进入上下文，
    /// 内层 `value.block_on` 再次 `enter` 同一份运行时（`scoped_tls` 允许重入）。
    ///
    /// 实施策略：创建 compio 运行时，在最外层 `rt.block_on` 中构造运行时值，
    /// 再用该值 `block_on` 一个返回常量表达式的 future，并把结果带出外层。
    ///
    /// 通过依据：外层 `rt.block_on` 的返回值等于 6 * 7 == 42，且整个过程没有
    /// panic（嵌套 `enter` 正常）。
    #[test]
    fn block_on_inside_runtime_returns_output() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            value.block_on(async { 6 * 7 })
        });

        assert_eq!(out, 42);
    }

    /// 目的：验证 `block_on` 阻塞等待期间，同运行时的其他任务仍会被调度——即「不影响
    /// 运行时调度」的契约。
    ///
    /// 实施策略：先用**本地作用域** `spawn_local` 投递一个后台任务（循环递增
    /// `Rc<Cell<usize>>` 计数器后返回计数值），再用同一个运行时值 `block_on` 去 await 该
    /// 任务的句柄。compio 的 JoinHandle 自身不会内联执行任务，它只注册 waker 并等 executor
    /// 把任务跑完；因此若 `block_on` 没有在等待期间 tick executor，这个 await 永远不会完成。
    ///
    /// 通过依据：整个场景放在独立线程中执行，并用 `recv_timeout` 限制等待时间。若 10 秒内
    /// 返回、句柄结果为 `Ok` 且返回值等于 1000，说明后台任务确实在 `block_on` 期间被驱动
    /// 执行了；若超时或线程 panic 则失败。计数器刻意用 `Rc<Cell<_>>`（`!Send`），一并钉住
    /// 「本地任务不跨线程」。
    #[test]
    fn block_on_drives_local_tasks() {
        const TARGET: usize = 1000;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let rt = CompioRuntime::new().unwrap();

            let result = rt.block_on(async {
                let value = crate::current();
                let scope = value.local_scope();
                let counter = Rc::new(Cell::new(0usize));
                let c = counter.clone();

                let handle = scope.spawn_local(async move {
                    for _ in 0..TARGET {
                        c.set(c.get() + 1);
                    }
                    c.get()
                });

                let got = value.block_on(async { handle.await.unwrap() });
                assert_eq!(counter.get(), TARGET);
                got
            });

            let _ = tx.send(result);
        });

        let result = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|e| panic!("测试失败：等待结果超时或线程 panic（{e:?}）"));
        assert_eq!(result, TARGET);
    }

    /// 目的：验证同一个运行时值上的多次 `block_on` 相互独立，不残留任何运行时
    /// 状态（每次调用都重新 `enter` 同一份运行时）。
    ///
    /// 实施策略：在同一个外层 `rt.block_on` 上下文中先后调用两次 `value.block_on`，
    /// 每次驱动不同的 future，再把两次结果相加。
    ///
    /// 通过依据：外层 `rt.block_on` 返回 1 + 2 + 3 + 4 == 10，说明两次调用都正常
    /// 工作且互不干扰。
    #[test]
    fn block_on_multiple_calls_are_independent() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = crate::current();
            let a = value.block_on(async { 1 + 2 });
            let b = value.block_on(async { 3 + 4 });
            a + b
        });

        assert_eq!(out, 10);
    }

    /// 目的：验证运行时值可以在 compio 上下文**之外**构造，并独立 `block_on`
    /// ——这是值化相对旧的环境式入口（依赖 `with_current`）新增的能力。
    ///
    /// 实施策略：先在当前线程（不在任何 compio 上下文内）`CompioRuntime::new()`，
    /// 用 `Runtime::with_runtime` 把它搬成运行时值，再直接调 `value.block_on`。
    ///
    /// 通过依据：返回 6 * 7 == 42 且不 panic；若实现仍走 `with_current`，这里会
    /// panic（`not in a compio runtime`）。
    #[test]
    fn with_runtime_block_on_outside_context() {
        let rt = CompioRuntime::new().unwrap();
        let value = Runtime::<{ crate::FULL }>::with_runtime(rt.clone());

        let out = value.block_on(async { 6 * 7 });

        assert_eq!(out, 42);
    }

    /// 目的：验证在没有任何 compio 运行时上下文的线程中**构造**运行时值会 panic
    /// ——`Runtime::current()` 需要环境运行时，该函数在没有环境运行时时必然 panic。
    /// 这固定了「上下文构造」与「搬句柄构造」两条入口的分工：要脱离上下文，必须
    /// 显式用 `with_runtime` 把运行时句柄搬进来。
    ///
    /// 实施策略：不创建也不进入任何 compio 运行时，直接在测试线程中调用
    /// `crate::current()`。
    ///
    /// 通过依据：测试按预期捕获 panic（`expected = "not in a compio runtime"`
    /// 匹配 compio 的 panic 文案）即为通过；若没有 panic，则测试失败。
    #[test]
    #[should_panic(expected = "not in a compio runtime")]
    fn current_outside_runtime_panics() {
        let _ = crate::current();
    }

    /// 目的：固定「表达式位置不能裸写 `Runtime::current()`」这条实测约束，并验证
    /// 两条**可用**出口确实等价：crate 级自由函数 [`crate::current`] 与显式
    /// turbofish。实测结论：`let value = Runtime::current();` 会在当前工具链
    /// （rustc 1.101.0-nightly）上触发
    /// `E0284: type annotations needed for Runtime<_>`——const 泛型默认值 `FULL`
    /// **不参与**函数调用返回位置的推断。因此自由函数不是锦上添花，而是表达式
    /// 位置的必需品（tokio crate 报告过同一现象）。
    ///
    /// 实施策略：在 compio 运行时上下文内分别用自由函数与
    /// `Runtime::<{ crate::FULL }>::current()` 各取一个值。
    ///
    /// 通过依据：两者都报告 `RuntimeTag::Compio`，且本测试能编译（反例见上方注释
    /// 里那条无法编译的写法）。
    #[test]
    fn current_free_function_matches_turbofish() {
        let rt = CompioRuntime::new().unwrap();

        rt.block_on(async {
            let a = crate::current();
            let b = Runtime::<{ crate::FULL }>::current();
            assert_eq!(a.tag(), abs_art::RuntimeTag::Compio);
            assert_eq!(b.tag(), abs_art::RuntimeTag::Compio);
        });
    }

    /// 目的：验证 Tag 模式下，声明了 `BLOCK_ON` 能力的 `Runtime<Caps>` 确实实现了
    /// `TrBlockOn`（编译期能力检查的正向用例）。
    ///
    /// 实施策略：在 compio 运行时上下文内，用
    /// `Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current()` 取得运行时值，再通过值方法
    /// `block_on` 驱动一个 future。
    ///
    /// 通过依据：返回值为 40 + 2 == 42；若 `HasBlockOn` 标记或条件化 trait impl 有误，
    /// 将无法编译。
    #[test]
    fn tagged_runtime_implements_block_on() {
        let rt = CompioRuntime::new().unwrap();

        let out = rt.block_on(async {
            let value = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
            value.block_on(async { 40 + 2 })
        });
        assert_eq!(out, 42);
    }
}
