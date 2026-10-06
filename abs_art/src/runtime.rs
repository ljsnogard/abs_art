//! 抽象运行时：**值化的能力 trait**。
//!
//! 本模块不依赖任何异步运行时，所有 trait 的具体实现都在组合 crate
//! （`abs_art-tokio` / `abs_art-compio` / `abs_art-smol`）中给出。
//!
//! # 为什么能力挂在**运行时值**上（v0.4 的核心改动）
//!
//! v0.3 的能力是**无 `self` 的关联函数**：`Runtime<CAPS>::delay(d)`、
//! `<S as TrLocalScope>::spawn_local(f)`。那套形状的代价是「能力与环境分离」：
//!
//! - 计时能力挂在**类型**上，而本地队列挂在**另一个值**（各后端的 `LocalScope`）上，
//!   于是同一个进程里存在两套运行时（测试二进制里 tokio 与 compio 并存）时，
//!   「这次 `spawn_local` 投到哪条队列」「这次 `now()` 读的是谁的时钟」**没有类型
//!   层面的绑定**，只能靠调用方自觉；
//! - 调用点必须先**命名**运行时类型（或作用域值类型），库侧因此被迫穿一个类型参数，
//!   而这个参数的唯一用途就是「指向那份环境」。
//!
//! v0.4 把运行时**值化**：能力变成 `&self` 方法，运行时的本地队列、计时源、
//! 时刻源都由**这个值**提供。于是：
//!
//! | 问题 | v0.3 | v0.4 |
//! | --- | --- | --- |
//! | 这次 `spawn_local` 投到哪条队列？ | 由你手里那个 `LocalScope` 决定 | 由你手里这个**运行时值**决定 |
//! | `delay` 与 `now` 同源吗？ | 一个是类型、一个可能是别处注入的值 | 同一个值，`TrTime: TrDelay + TrClock` |
//! | 库侧要写什么？ | 类型参数 `Rt` + 作用域值 `S` | 只有一个运行时**值** `&R` |
//!
//! 代价是运行时值不再是「处处可写的 ZST」：它必须由持有环境的一方**构造出来**
//! （tokio 的 `Handle` / `LocalSet`、smol 的 `LocalExecutor`），并像其它资源值一样
//! 被传递。这是有意的——「哪个运行时」本来就是一个运行期事实，把它写进类型标签
//! 只会让类型正确、调用点错误（见 v0.3 的 `SPAWN_LOCAL` 位讨论）。
//!
//! # 能力位仍然只管「声明」
//!
//! [`crate::caps`] 的位掩码机制不变：`Runtime<CAPS>` 只是**值类型的形状参数**，
//! 掩码决定这个值实现了哪些能力 trait。值化不改变「想用就得写下来」这条设计意图。

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
    /// smol 后端的 `spawn_local` 任务：其本地执行器随运行时值存活（值的
    /// `run_until` 驱动执行器），detach 消费句柄后执行器仍归运行时值所有，
    /// 因此只要运行时值还活着且仍被驱动，本地任务就能继续推进。
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

/// 让当前线程阻塞等待一个异步任务完成，同时驱动本运行时值拥有的本地队列。
///
/// 被等待的 future 类型 `F` 是 [`block_on`](Self::block_on) 的方法级泛型参数。
/// 由于 `F` 不进 trait，`F` 不需要 `'static`——可以借用当前栈帧上的数据
/// （见 `abs_art-demo` 的 `cap_block_on` 示例）。
///
/// # 为什么它同时是「本地队列的阻塞驱动入口」
///
/// v0.3 把「阻塞等待」挂在运行时类型上、把「驱动本地队列」挂在作用域值上，
/// 于是调用方要先想清楚「我要驱动谁」。值化之后两者是同一件事：**这个运行时值
/// 拥有的东西，由这次的 `block_on` 一起驱动**——本地队列、计时器都归它。
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
    /// 阻塞当前线程，等待 `f` 完成并返回其结果，期间持续驱动本值的本地队列。
    fn block_on<F>(&self, f: F) -> F::Output
    where
        F: Future;
}

/// 值化的「本地作用域」：**运行时值自己**就是本地队列的持有者与驱动点。
///
/// # 与能力位 [`SPAWN_LOCAL`](crate::SPAWN_LOCAL) 的分工
///
/// 本地投递仍然是「位 + 值」两件事，但**值的那一半换人了**：
///
/// | | 回答的问题 | 载体 |
/// | --- | --- | --- |
/// | 能力位 `SPAWN_LOCAL` | 你**声明**了没有？ | `Runtime<CAPS>` 的类型级标记 |
/// | 本 trait 的实现 | 你**拿到**了没有？ | 运行时值本身 |
///
/// v0.3 的「值」是一个**独立的作用域对象**（各后端的 `LocalScope`），它可以脱离
/// 运行时类型单独存在、单独传递、单独驱动；v0.4 把它并回运行时值，因为那条环境
/// 前提（「此刻真的有本地队列，且有人驱动它」）本来就属于**这个运行时**，
/// 不属于任何一个可以被复制来复制去的对象。
///
/// # 实现契约
///
/// 1. [`spawn_local`](Self::spawn_local) 投递的任务，其推进**不得依赖
///    `Handle` 被 poll**——只要运行时值还活着且仍被驱动，任务就应当持续运行；
/// 2. [`TrJoinHandle::detach`] 之后任务**继续运行**：本地队列归运行时值所有；
/// 3. [`run_until`](Self::run_until) 在等待传入 future 期间，必须持续驱动本地队列；
/// 4. 阻塞驱动入口不在这里重复提供——它就是 [`TrBlockOn::block_on`]。
///
/// # Examples
///
/// 泛型库侧（不依赖任何后端，`R` 由最终二进制给出**值**）：
///
/// ```rust
/// use abs_art::TrLocalScope;
///
/// async fn run_local<R>(rt: &R) -> u32
/// where
///     R: TrLocalScope,
/// {
///     // 同一个约束即可投递多种（含调用点无法命名的）!Send future
///     let rc = std::rc::Rc::new(1u32);
///     rt.spawn_local(async move { *rc }).await.unwrap()
/// }
/// ```
pub trait TrLocalScope {
    /// 本地任务句柄类型，由组合 crate 给出，按 future 的输出类型 `T` 参数化。
    type Handle<T>: TrJoinHandle<T>
    where
        T: 'static;

    /// 把 `future` 投递到**本值**的本地队列，返回句柄。
    fn spawn_local<F>(&self, future: F) -> Self::Handle<<F as Future>::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static;

    /// 异步驱动入口：驱动**本值**的本地队列直到 `future` 完成。
    ///
    /// 返回的 future 需要放在「已处于该后端运行时上下文」的位置 await；
    /// 对 compio 这类运行时自己驱动队列的后端，它等价于直接 await `future`。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
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
