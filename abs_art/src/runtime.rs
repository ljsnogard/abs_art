//! 抽象运行时标签与能力 trait。
//!
//! 本模块不依赖任何异步运行时，所有 trait 的具体实现都在组合 crate
//! （`abs_art-tokio` / `abs_art-compio` / `abs_art-smol`）中给出。

use core::future::Future;

/// 抽象运行时标签。
///
/// 每个组合 crate 对应一个具体的变体（例如 [`abs_art_tokio`] 对应
/// [`Runtime::Tokio`]），本 crate 本身不实现任何运行时行为。
///
/// [`abs_art_tokio`]: https://docs.rs/abs_art-tokio
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RuntimeTag {
    /// compio 运行时。
    Compio,
    /// smol 运行时。
    Smol,
    /// tokio 运行时。
    Tokio,
}

pub trait TrAsyncRuntime {
    type JoinHandle<T>: TrJoinHandle<T> where T: 'static;

    fn about() -> RuntimeTag;
}

pub trait TrJoinHandle<T>
where
    Self: Future<Output = Result<T, Self::JoinErr>>
{
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
    /// smol 后端的 `spawn_local` 任务：其本地执行器随句柄存活（句柄 poll 时
    /// 驱动执行器），detach 消费句柄后执行器被销毁，本地任务无法继续推进
    /// （等同取消）。因此 smol 的 `detach` 只对 `spawn`（全局执行器）任务有
    /// 完整语义。
    fn detach(self);
}

/// 运行时可以把任务投递到全局（跨线程）工作窃取队列。
///
/// 被 spawn 的 future 类型 `F` 是 [`spawn`](Self::spawn) 的**方法级泛型参数**，
/// 不出现在 trait 上。于是一个运行时类型只需实现本 trait 一次，就能 spawn
/// 任意多种不同的 future（包括调用点无法命名的 `async {}` 块），库侧写一个
/// `Rt: TrSpawnSend` 约束即可覆盖全部任务类型。
///
/// 返回的句柄经 GAT [`JoinHandle<T>`](Self::JoinHandle) 按 future 的**输出类型**
/// `T` 参数化，因此本基础 crate 不持有任何与具体运行时相关的类型。
///
/// # Examples
///
/// 泛型库侧（不依赖任何后端，`Rt` 由最终二进制实例化）：
///
/// ```rust
/// use abs_art::TrSpawnSend;
///
/// async fn run_two<Rt>() -> u32
/// where
///     Rt: TrSpawnSend,
/// {
///     // 同一个约束即可 spawn 两种不同的（其中一个还无法命名的）future
///     let a = Rt::spawn(async { 1u32 }).await.unwrap();
///     let b = Rt::spawn(async move { a + 1 }).await.unwrap();
///     b
/// }
/// ```
///
/// # Panics
///
/// 是否 panic 由具体后端的实现决定，本 trait 不作承诺。
pub trait TrSpawnSend {
    /// 任务句柄类型，由组合 crate 给出（例如 `abs_art-tokio` 中的
    /// [`JoinHandle`]），按 future 的输出类型 `T` 参数化。
    ///
    /// [`JoinHandle`]: https://docs.rs/abs_art-tokio
    type JoinHandle<T>: TrJoinHandle<T> where T: 'static;

    /// 把 `future` 投递到全局工作队列，返回句柄 `H`。
    ///
    /// # Errors
    ///
    /// 句柄只有在被 `await` 时才可能产出错误（任务 panic 等），投递本身
    /// 返回句柄而不返回 `Result`。
    fn spawn<F>(future: F) -> Self::JoinHandle<<F as Future>::Output>
    where
        F: Future + Send + 'static,
        <F as Future>::Output: Send + 'static;
}

/// 运行时可以把任务投递到线程本地工作队列。
///
/// 与 [`TrSpawnSend`] 的形状相同：future 类型 `F` 是 [`spawn_local`](Self::spawn_local)
/// 的方法级泛型参数。区别只在约束——本 trait 不要求 `F: Send`，因此可以承载
/// 捕获 `Rc` 等 `!Send` 数据的任务。
///
/// # Examples
///
/// ```rust
/// use abs_art::TrSpawnLocal;
///
/// async fn run_local<Rt>() -> u32
/// where
///     Rt: TrSpawnLocal,
/// {
///     let rc = std::rc::Rc::new(1u32);
///     Rt::spawn_local(async move { *rc }).await.unwrap()
/// }
/// ```
pub trait TrSpawnLocal {
    /// 任务句柄类型，由组合 crate 给出，按 future 的输出类型 `T` 参数化。
    type JoinHandle<T>: TrJoinHandle<T> where T: 'static;

    /// 把 `future` 投递到线程本地工作队列，返回句柄 `H`。
    fn spawn_local<F>(future: F) -> Self::JoinHandle<<F as Future>::Output>
    where
        F: Future + 'static,
        <F as Future>::Output: 'static;
}

/// 运行时可以把阻塞函数投递到阻塞线程池。
///
/// 方法级泛型有**两个**：闭包类型 `F` 与闭包输出类型 `T`；由于 `F` 不进 trait，
/// 句柄 GAT [`JoinHandle<T>`](Self::JoinHandle) 改为按输出类型 `T` 参数化
/// （v0.2 的写法把 `T` 放在 trait 参数上、句柄关联类型不具参，无法在 `F`
/// 移出 trait 后继续表达）。
///
/// # Examples
///
/// ```rust
/// use abs_art::TrSpawnBlocking;
///
/// async fn compute<Rt>() -> u32
/// where
///     Rt: TrSpawnBlocking,
/// {
///     Rt::spawn_blocking(|| 6 * 7).await.unwrap()
/// }
/// ```
pub trait TrSpawnBlocking {
    /// 任务句柄类型，由组合 crate 给出，按阻塞函数输出类型 `T` 参数化。
    type JoinHandle<T>: TrJoinHandle<T> where T: 'static;

    /// 把阻塞函数 `f` 投递到阻塞线程池，返回句柄 `H`。
    fn spawn_blocking<F, T>(f: F) -> Self::JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static;
}

/// 让当前线程阻塞等待一个异步任务完成，同时不影响运行时的调度。
///
/// 被等待的 future 类型 `F` 是 [`block_on`](Self::block_on) 的方法级泛型参数。
/// 由于 `F` 不进 trait，`F` 不需要 `'static`——可以借用当前栈帧上的数据
/// （见 `abs_art-demo` 的 `cap_block_on` 示例）。
///
/// # Examples
///
/// ```rust
/// use abs_art::TrBlockOn;
///
/// fn len_of_stack_string<Rt>() -> usize
/// where
///     Rt: TrBlockOn,
/// {
///     let s = String::from("hello");
///     // 借用局部 `s` 的 future 不是 'static，仍然可以 block_on
///     Rt::block_on(async { s.len() })
/// }
/// ```
pub trait TrBlockOn {
    /// 阻塞当前线程，等待 `f` 完成并返回其结果。
    fn block_on<F>(f: F) -> F::Output
    where
        F: Future;
}

/// 暂停当前执行上下文一段时间。
pub trait TrDelay {
    /// 返回一个等待 `duration` 之后完成的 future。
    fn delay(duration: core::time::Duration) -> impl Future<Output = ()>;
}
