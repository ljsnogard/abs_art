//! 抽象运行时：**值化的能力 trait**。
//!
//! 本模块不依赖任何异步运行时，所有 trait 的具体实现都在组合 crate
//! （`abs_art-tokio` / `abs_art-compio` / `abs_art-smol`）中给出。
//!
//! # 两个概念，两张皮：**运行时值**与**本地作用域**
//!
//! 本版把运行时值化了：能力变成 `&self` 方法（`rt.spawn(..)` / `rt.delay(..)` /
//! `rt.now()`）。但**本地队列不属于运行时值**——它是**线程独占**的资源，必须单独成为
//! 一个值：[`TrLocalScope`]（各后端的 `LocalScope`）。
//!
//! | 关切 | 挂在哪 | 为什么 |
//! | --- | --- | --- |
//! | 投递到全局工作队列（`spawn`） | 运行时**值** | 那份队列可以跨线程共享，值（如 tokio 的 `Handle`）正是它的把手 |
//! | 阻塞等待 / 周期源 / 时刻 | 运行时**值** | 它们与「在哪个线程上调度」无关 |
//! | 投递 `!Send` 任务（`spawn_local`） | [`TrLocalScope`] | 队列绑定**线程**：tokio 的 `LocalSet`、smol 的 `LocalExecutor` 都 `!Send`，必须由持有者驱动 |
//!
//! ## 为什么本地队列不能塞进运行时值（实测教训）
//!
//! 曾经把它并进运行时值，得到两个直接后果：
//!
//! 1. **值被迫 `!Send`**：tokio 的 `Handle` 本来是 `Send + Sync`，但一旦值里再装一个
//!    `Rc<LocalSet>`，整个值就不能跨线程传了——「全局 spawn 的能力」被一个与它无关的
//!    队列绑住。
//! 2. **两种生命周期被绑死**：句柄可以共享、可以长期活着；队列必须绑定线程、必须由
//!    持有者驱动。合成一个值之后，只能「值亡则队列亡」。
//!
//! 三个后端的真实形状并不一致（这也是「如实表达」的一部分）：
//!
//! | 后端 | 队列在哪 | 作用域是什么 |
//! | --- | --- | --- |
//! | tokio | 调用方的 `LocalSet`（与 `Handle` 可分离） | 持有 `Rc<LocalSet>` 的值 |
//! | smol | 调用方的 `LocalExecutor`（可分离） | 持有 `Rc<LocalExecutor>` 的值 |
//! | compio | **运行时实例自己的**执行器（不可分离） | 零大小标记：队列归当前运行时 |
//!
//! ## 作用域怎么来
//!
//! 只能从运行时值取得：`Runtime<CAPS>::local_scope()`，且要求 `CAPS` 含
//! [`SPAWN_LOCAL`](crate::SPAWN_LOCAL)。这一步就是「声明 → 取得」的串联点，也保证了
//! 作用域不会脱离运行时凭空出现（原设计那种「作用域与运行时无关」的串由此消失）。
//!
//! [`TrLocalScope`] **不**提供计时与时刻：[`TrDelay`] / [`TrClock`](crate::TrClock) /
//! [`TrTime`](crate::TrTime) 都在
//! 运行时值上——它们与本地调度无关，调用者从 `rt` 上取用即可。
//!
//! # 能力位仍然只管「声明」
//!
//! [`crate::caps`] 的位掩码机制不变：`Runtime<CAPS>` 是值类型的形状参数，掩码决定
//! 这个值实现了哪些能力 trait、以及能不能经 `local_scope()` 取得作用域。

use core::future::Future;

/// 抽象运行时标签（身份枚举）。
///
/// 每个组合 crate 对应一个具体的变体（例如 `abs_art-tokio` 对应
/// [`RuntimeTag::Tokio`]），本 crate 本身不实现任何运行时行为。
/// 值化之后，它由 [`TrAsyncRuntime::about`] 在**运行时值**上报告。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RuntimeTag {
    /// compio 运行时。
    Compio,
    /// smol 运行时。
    Smol,
    /// tokio 运行时。
    Tokio,
}

/// 运行时值的公共契约：它能交出任务句柄类型，并能自报身份。
///
/// 值化之后，这是唯一一个「所有运行时值都必须实现」的元能力 trait；
/// 其余能力（`spawn` / `delay` / …）各自独立，由能力位门控。
pub trait TrAsyncRuntime {
    /// 任务句柄类型，由组合 crate 给出（例如 `abs_art-tokio` 的 `JoinHandle`），
    /// 按 future 的输出类型 `T` 参数化。
    type JoinHandle<T>: TrJoinHandle<T>
    where
        T: 'static;

    /// 报告本值的运行时身份。
    fn about(&self) -> RuntimeTag;
}

/// 任务句柄：可以被 `await` 出结果，也可以被 `detach`。
///
/// 超 trait 把它钉成 `Future<Output = Result<T, Self::JoinErr>>`，于是泛型代码可以
/// **直接 `.await` 泛型句柄**，不必知道它是谁。
pub trait TrJoinHandle<T>
where
    Self: Future<Output = Result<T, Self::JoinErr>>,
{
    /// 任务失败（panic / 取消）时的错误类型。
    type JoinErr: core::error::Error;

    /// 让任务脱离句柄，继续在后台运行，不再能 join / await 其结果。
    ///
    /// `detach` 消费掉句柄本身：调用后任务仍在运行时的队列里继续推进，
    /// 但调用方失去了等待它完成的能力（任务完成后其输出会被丢弃）。
    ///
    /// # 三个后端的支持情况（可行性结论）
    ///
    /// - **tokio**：无原生 `detach` 方法（`JoinHandle` 只有 `abort` /
    ///   `is_finished` / `abort_handle` / `id`）；官方文档明确「drop 句柄
    ///   即 detach」——任务继续在后台运行。实现为丢弃句柄即可，语义正确。
    /// - **smol**：有原生 `Task::detach`（底层 async-task：置 detached 标志
    ///   后 forget）。**不能**靠 drop 实现：async-task 的 `Task` 在 drop 时
    ///   会 `set_canceled()` 取消任务。
    /// - **compio**：有原生 `JoinHandle::detach`（丢弃任务句柄而不取消）。
    ///   **不能**靠 drop 实现：compio 的 `JoinHandle` 在 drop 时会
    ///   `cancel(true)` 取消任务。
    ///
    /// # 已知限制
    ///
    /// smol 后端的 `spawn_local` 任务：其本地执行器随**作用域值**存活
    /// （[`TrLocalScope::run_until`] / [`TrLocalScope::block_on`] 驱动执行器），
    /// detach 消费句柄后执行器仍归作用域所有，因此只要作用域还活着且仍被驱动，
    /// 本地任务就能继续推进。
    fn detach(self);
}

/// 运行时值可以把任务投递到全局（跨线程）工作队列。
///
/// 被 spawn 的 future 类型 `F` 是 [`spawn`](Self::spawn) 的**方法级泛型参数**，
/// 不出现在 trait 上。于是一个运行时类型只需实现本 trait 一次，就能 spawn
/// 任意多种不同的 future（包括调用点无法命名的 `async {}` 块），库侧写一个
/// `R: TrSpawnSend` 约束即可覆盖全部任务类型。
///
/// # Examples
///
/// 泛型库侧（不依赖任何后端，`R` 由最终二进制给出**值**）：
///
/// ```rust
/// use abs_art::TrSpawnSend;
///
/// async fn run_two<R>(rt: &R) -> u32
/// where
///     R: TrSpawnSend,
/// {
///     // 同一个约束即可 spawn 两种不同的（其中一个还无法命名的）future
///     let a = rt.spawn(async { 1u32 }).await.unwrap();
///     let b = rt.spawn(async move { a + 1 }).await.unwrap();
///     b
/// }
/// ```
///
/// # Panics
///
/// 是否 panic 由具体后端的实现决定，本 trait 不作承诺。
pub trait TrSpawnSend {
    /// 任务句柄类型，由组合 crate 给出，按 future 的输出类型 `T` 参数化。
    type JoinHandle<T>: TrJoinHandle<T>
    where
        T: 'static;

    /// 把 `future` 投递到全局工作队列，返回句柄。
    ///
    /// # Errors
    ///
    /// 句柄只有在被 `await` 时才可能产出错误（任务 panic 等），投递本身
    /// 返回句柄而不返回 `Result`。
    fn spawn<F>(&self, future: F) -> Self::JoinHandle<<F as Future>::Output>
    where
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static;
}

/// 运行时值可以把阻塞函数投递到阻塞线程池。
///
/// 方法级泛型有**两个**：闭包类型 `F` 与闭包输出类型 `T`；由于 `F` 不进 trait，
/// 句柄 GAT [`JoinHandle<T>`](Self::JoinHandle) 改为按输出类型 `T` 参数化。
///
/// # Examples
///
/// ```rust
/// use abs_art::TrSpawnBlocking;
///
/// async fn compute<R>(rt: &R) -> u32
/// where
///     R: TrSpawnBlocking,
/// {
///     rt.spawn_blocking(|| 6 * 7).await.unwrap()
/// }
/// ```
pub trait TrSpawnBlocking {
    /// 任务句柄类型，由组合 crate 给出，按阻塞函数输出类型 `T` 参数化。
    type JoinHandle<T>: TrJoinHandle<T>
    where
        T: 'static;

    /// 把阻塞函数 `f` 投递到阻塞线程池，返回句柄。
    fn spawn_blocking<F, T>(&self, f: F) -> Self::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static;
}

/// 让当前线程阻塞等待一个异步任务完成（不涉及本地队列）。
///
/// 被等待的 future 类型 `F` 是 [`block_on`](Self::block_on) 的方法级泛型参数。
/// 由于 `F` 不进 trait，`F` 不需要 `'static`——可以借用当前栈帧上的数据
/// （见 `abs_art-demo` 的 `cap_block_on` 示例）。
///
/// # 它与作用域的 `block_on` 的分工
///
/// 本地队列不归运行时值所有（见 [`TrLocalScope`]），所以本方法**只等待**，
/// 不驱动任何 `!Send` 任务的队列。要「阻塞等待并驱动本地队列」，用
/// [`TrLocalScope::block_on`]。
///
/// # Examples
///
/// ```rust
/// use abs_art::TrBlockOn;
///
/// fn len_of_stack_string<R>(rt: &R) -> usize
/// where
///     R: TrBlockOn,
/// {
///     let s = String::from("hello");
///     // 借用局部 `s` 的 future 不是 'static，仍然可以 block_on
///     rt.block_on(async { s.len() })
/// }
/// ```
pub trait TrBlockOn {
    /// 阻塞当前线程，等待 `f` 完成并返回其结果。
    fn block_on<F>(&self, f: F) -> F::Output
    where
        F: Future;
}

/// 本地作用域：**线程独占**的本地队列，由它负责投递与驱动 `!Send` 任务。
///
/// # 它为什么是独立的值
///
/// 本地队列绑定**线程**（tokio 的 `LocalSet`、smol 的 `LocalExecutor` 都 `!Send`），
/// 与「运行时」是两件事：运行时把手可以跨线程共享、可以长期活着，而队列必须由持有者
/// 在**创建它的线程**上驱动。因此本 trait 的宿主是一个独立的值，不并进运行时值
/// （理由与实测见模块文档）。
///
/// compio 是例外中的例外：它的执行器就在运行时实例里、不可分离，所以它的作用域是
/// **零大小**的标记——投递走的仍是当前运行时的队列，`run_until` 等价于直接 await。
///
/// # 与能力位 [`SPAWN_LOCAL`](crate::SPAWN_LOCAL) 的分工
///
/// | | 回答的问题 | 载体 |
/// | --- | --- | --- |
/// | 能力位 `SPAWN_LOCAL` | 你**声明**了没有？ | `Runtime<CAPS>` 的类型级标记 |
/// | 本 trait 的实现 | 你**拿到**了没有？ | 各后端的 `LocalScope` 值 |
///
/// 集成方经各后端的 `Runtime<CAPS>::local_scope()` 取得作用域，而该关联函数要求
/// `CAPS` 含本位——于是「开始用本地投递」这个动作必然在代码里留下痕迹。
///
/// # 它**不**提供计时与时刻
///
/// [`TrDelay`] / [`TrClock`](crate::TrClock) / [`TrTime`](crate::TrTime)
/// 都实现在**运行时值**上，因为它们与本地调度
/// 无关；需要等待/时刻的代码写 `R: TrTime` 并从运行时值上调用。作用域只回答一个问题：
/// 「`!Send` 任务投到哪、由谁驱动」。
///
/// # 实现契约
///
/// 1. [`spawn_local`](Self::spawn_local) 投递的任务，其推进**不得依赖句柄被 poll**——
///    只要作用域还活着且处于被驱动状态，任务就应当持续运行；
/// 2. [`TrJoinHandle::detach`] 之后任务**继续运行**：队列归作用域所有；
/// 3. [`run_until`](Self::run_until) / [`block_on`](Self::block_on) 在等待传入 future
///    期间，必须持续驱动本地队列。
///
/// # Examples
///
/// 泛型库侧（不依赖任何后端，`S` 由最终二进制给出**值**）：
///
/// ```rust
/// use abs_art::TrLocalScope;
///
/// async fn run_local<S>(scope: &S) -> u32
/// where
///     S: TrLocalScope,
/// {
///     // 同一个约束即可投递多种（含调用点无法命名的）!Send future
///     let rc = std::rc::Rc::new(1u32);
///     scope.spawn_local(async move { *rc }).await.unwrap()
/// }
/// ```
pub trait TrLocalScope {
    /// 本地任务句柄类型，由组合 crate 给出，按 future 的输出类型 `T` 参数化。
    type Handle<T>: TrJoinHandle<T>
    where
        T: 'static;

    /// 把 `future` 投递到**本作用域**的本地队列，返回句柄。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<<F as Future>::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static;

    /// 异步驱动入口：驱动本作用域的本地队列直到 `future` 完成。
    ///
    /// 返回的 future 需要放在「已处于该后端运行时上下文」的位置 await；
    /// 对 compio 这类运行时自己驱动队列的后端，它等价于直接 await `future`。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future;

    /// 阻塞驱动入口：阻塞当前线程，驱动本作用域的本地队列直到 `future` 完成。
    ///
    /// 各后端的先决条件与其 [`TrBlockOn`] 实现保持一致：
    ///
    /// - **tokio**：需要多线程运行时，且调用点已处于运行时上下文内
    ///   （实现走 `block_in_place` + `Handle::block_on`）；
    /// - **compio**：运行时自己 `enter`，无额外先决条件；
    /// - **smol**：无先决条件。
    ///
    /// # Panics
    ///
    /// 不满足上述先决条件时 panic；具体由各后端实现决定，本 trait 不作统一承诺。
    ///
    /// # 它和 [`TrBlockOn::block_on`] 的区别
    ///
    /// 两者宿主类型不同（作用域 vs 运行时值），语义也不同：
    /// `scope.block_on(f)` 在等待期间**驱动本地队列**；`rt.block_on(f)` 只等待，
    /// 不涉及任何本地队列。
    fn block_on<F>(&self, future: F) -> <F as Future>::Output
    where
        F: Future;
}

/// 暂停当前执行上下文一段时间。
///
/// 这是计时能力的**最小原语**；周期与超时见 [`crate::time::TrTime`]（它是本 trait 的
/// 超 trait，**不**重复提供「睡眠」这个能力），绝对时刻见
/// [`crate::time::TrClock`]（同样是它的超 trait）。
///
/// # Examples
///
/// ```rust
/// use core::time::Duration;
///
/// use abs_art::TrDelay;
///
/// async fn nap<R>(rt: &R) -> R::Delay
/// where
///     R: TrDelay,
/// {
///     rt.delay(Duration::from_millis(1))
/// }
/// ```
pub trait TrDelay {
    /// [`TrDelay::delay`] 返回的 future 类型（由后端给出**具体类型**）。
    ///
    /// 是关联类型而不是 `impl Future`：调用方因此能**命名**它——可以存进结构体、
    /// 可以写 `where Self::Delay: Send`，自动 trait 不再被不透明类型挡住。
    type Delay: Future<Output = ()>;

    /// 返回一个等待 `duration` 之后完成的 future。
    fn delay(&self, duration: core::time::Duration) -> Self::Delay;
}
