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
//! 三个后端的形状以 **compio 为基准**对齐：
//!
//! | 后端 | 队列在哪 | 作用域是什么 |
//! | --- | --- | --- |
//! | tokio | **本线程的 `thread_local!`**（`Rc<LocalSet>`） | 那条队列的别名（`!Send`、可 `Clone`） |
//! | smol | **本线程的 `thread_local!`**（`Rc<LocalExecutor<'static>>`） | 那条队列的别名（`!Send`、可 `Clone`） |
//! | compio | **运行时实例自己的**执行器（不可分离，运行时本身线程绑定） | 那份运行时的别名（`!Send`、可 `Clone`） |
//!
//! 由此得到三条对外可依赖的性质：`Clone` 只是**多加一个别名**（不是新建队列）；
//! 同一线程上多次 `local_scope()` 拿到的是**同一条**队列；类型是 `!Send`，跨线程只能
//! 在目标线程上另取。「同一线程多条本地队列」这套 tokio 用法被**显式排除**。
//!
//! ## 作用域的阻塞入口是 `block_on_local`，不是运行时原语
//!
//! `TrLocalScope` 有三条方法：[`spawn_local`](TrLocalScope::spawn_local)（投递）、
//! [`run_until`](TrLocalScope::run_until)（异步驱动本线程队列），以及
//! [`block_on_local`](TrLocalScope::block_on_local)（**同步**等到 future 完成，等待期间
//! 持续驱动本线程队列）。
//!
//! 早先本 trait 上确实有一条 `block_on`，但它把「怎么阻塞」整个交给各后端，于是同一个
//! 名字在三处的承诺互不相同：tokio 走 `block_in_place`（在 `LocalSet` 内被 tokio 自己
//! 禁止，源码注释原文「in a LocalSet, where it is _not_ okay to block」）、compio 顺带
//! 驱动整个运行时、smol 只驱动自己那条队列。那条方法因此被删除。
//!
//! 现在的 [`block_on_local`](TrLocalScope::block_on_local) 不是把老路捡回来：它
//! **不使用任何运行时的阻塞原语**，而是「自己把本线程队列驱动起来 + 把线程挂起/自己
//! tick」：
//!
//! | 后端 | 等待期间做什么 | 本线程的 IO |
//! | --- | --- | --- |
//! | tokio | `LocalSet::run_until` 驱动队列 + 纯 park | 不推进（IO driver 依附 `block_on`） |
//! | smol | `LocalExecutor::run` 驱动队列 + 纯 park | 不推进（`async-io` 反应器同此） |
//! | compio | 自己 tick（`Runtime::run` + `poll_with`，与自己 `block_on` 同一套循环） | 照常推进 |
//!
//! 三者的**接口与可观察语义一致**（同步等待 + 不饿死本地队列 + 可在本地队列的驱动栈内
//! 调用）；差异只剩「本线程的 IO 由谁推」——那是各运行时自身的性质，如实写在表里，
//! 调用方按它决定线程布局。
//!
//! ## 作用域怎么来
//!
//! 只能从运行时值取得：`Runtime<CAPS>::local_scope()`，且要求 `CAPS` 含
//! [`SPAWN_LOCAL`](crate::SPAWN_LOCAL)。这一步就是「声明 → 取得」的串联点，也保证了
//! 作用域不会脱离运行时凭空出现（原设计那种「作用域与运行时无关」的串由此消失）。
//! 又因为队列在 `thread_local!` 里，「取得」与「克隆」都只是拿别名，不会多出一条队列。
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
    /// tokio / smol 后端的 `spawn_local` 任务：本地队列在**本线程的 `thread_local!`**
    /// 里（由 [`TrLocalScope::run_until`] 驱动），寿命与线程相同。detach 消费句柄
    /// 不影响队列，因此只要本线程仍在驱动它，本地任务就能继续推进；线程退出时队列
    /// 一并销毁，残留任务随之消失。
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
/// # 它**不**驱动本地队列
///
/// 本地队列不归运行时值所有（见 [`TrLocalScope`]），所以本方法**只等待**，
/// 不驱动任何 `!Send` 任务的队列。需要「等待期间继续驱动本线程的本地队列」时用作用域的
/// [`TrLocalScope::block_on_local`]——它自己驱动队列、不借运行时的阻塞原语；而本方法与
/// [`TrLocalScope::run_until`] 的组合 `rt.block_on(scope.run_until(f))` 只在**不在本地
/// 队列驱动栈内**时成立，tokio 在 `LocalSet` 内会直接 panic。
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

/// 本地作用域：**本线程那条本地队列的别名**，由它负责投递与驱动 `!Send` 任务。
///
/// # 它是线程本地对象的别名，不是队列的所有者
///
/// 队列存放在**本线程的 `thread_local!`** 里（tokio 的 `LocalSet`、smol 的
/// `LocalExecutor` 都是 `!Send`），`LocalScope` 只是它的一个别名：
///
/// - `Clone` 只增加一个别名，**不是**新建一条队列；
/// - 同一线程上多次 `Runtime::local_scope()` 得到的是**同一条**队列；
/// - 类型是 `!Send`：想在别的线程上投递，就到**那条线程上**另取一个作用域；
/// - 队列寿命与线程相同：线程退出即销毁，未完成的任务随之消失。
///
/// 「同一线程多条本地队列」这套 tokio 用法被**显式排除**：家族只对齐 compio 的语义
/// （队列与线程绑定、克隆即别名）。compio 的队列在运行时实例里、不可分离，它天然
/// 满足这套语义，是本次对齐的基准。
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
/// # 它的阻塞入口是 [`block_on_local`](Self::block_on_local)
///
/// 作用域**有**一条阻塞入口，但它不是运行时的阻塞原语：它自己驱动本线程队列，再把线程
/// 挂起（tokio / smol 的纯 park），或在 compio 上自己 tick 执行器与 IO。因此它可以在
/// **本地队列的驱动栈内**调用——这正是 tokio 的 `block_in_place` 做不到的事。
///
/// 需要「只等待、不驱动本地队列」时，用运行时值的 [`TrBlockOn::block_on`]。
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
///    只要本线程仍在驱动这条队列，任务就应当持续运行；
/// 2. [`TrJoinHandle::detach`] 之后任务**继续运行**：队列归本线程所有；
/// 3. [`run_until`](Self::run_until) 在等待传入 future 期间，必须持续驱动本地队列；
/// 4. [`block_on_local`](Self::block_on_local) 在阻塞期间同样必须持续驱动本地队列，
///    且**不得**依赖各运行时的阻塞原语（tokio 的 `block_in_place` 在 `LocalSet` 内被
///    禁止，一旦用了，调用点位于本地队列驱动栈内时就会 panic 或停摆）。
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

    /// 驱动**本线程的本地队列**，直到 `future` 完成。
    ///
    /// 这是作用域上唯一的驱动入口（抽象层不再提供阻塞版本，理由见类型文档）。
    /// 三个后端对这条语义的支持方式不同，调用方要把握的契约是
    /// **「返回的 future 由谁 poll」**：
    ///
    /// | 后端 | `run_until(f)` 实际做什么 | 返回的 future 放在哪里 await |
    /// | --- | --- | --- |
    /// | tokio | 驱动本线程 `thread_local!` 里的 `LocalSet`（`LocalSet::run_until`） | 必须已处于 tokio 运行时上下文内 |
    /// | smol | 驱动本线程 `thread_local!` 里的 `LocalExecutor`（`LocalExecutor::run`） | 任意能驱动 `async-io` 反应器的位置 |
    /// | compio | **等价于直接 await `f`**：队列由所在运行时的 `block_on`/`wait` 自己 tick | 必须已处于 compio 运行时上下文内（本方法自己不驱动队列） |
    ///
    /// compio 那一行不是偷懒：它的执行器就在运行时实例里、由运行时自己驱动，
    /// 「驱动队列直到 `f` 完成」与「await `f`」本来就是同一件事。
    ///
    /// # 嵌套与阻塞
    ///
    /// `run_until` 可以嵌套 `run_until`（外层驱动本线程队列的同时，内层继续驱动同一条
    /// 队列）。但**不得**在其内部调用运行时的阻塞原语：tokio 的 `block_in_place` 在
    /// `LocalSet` 内会 panic。需要**同步**等待时用
    /// [`block_on_local`](Self::block_on_local)——它自己驱动队列、不借运行时原语，
    /// 因此在本地驱动栈内也成立。
    fn run_until<F>(&self, future: F) -> impl Future<Output = <F as Future>::Output>
    where
        F: Future;

    /// 阻塞当前线程直到 `future` 完成；等待期间**持续驱动本线程的本地队列**。
    ///
    /// 这是「同步等一个由本地任务供料的 future」的入口：调用方写的是同步代码
    /// （例如 `std::io::Read`），而数据要靠投递在本线程队列上的循环搬进来时，用本方法
    /// 原地等待——队列里的其它任务不会被饿死。
    ///
    /// # 它和 [`TrBlockOn::block_on`] 不是一回事
    ///
    /// | | 驱动本地队列 | 能在本地驱动栈内调用 | 本线程的 IO |
    /// | --- | --- | --- | --- |
    /// | [`TrBlockOn::block_on`] | **不** | tokio 上**不能**（`block_in_place` 被禁止） | tokio 让渡 worker，全局任务照跑 |
    /// | `block_on_local` | **会** | **能**（不用任何运行时阻塞原语） | tokio / smol 停；compio 照推 |
    ///
    /// # 各后端的做法
    ///
    /// | 后端 | 实现 |
    /// | --- | --- |
    /// | tokio | `LocalSet::run_until` 驱动队列，外层是**纯 park** 执行器（`park_timeout`） |
    /// | smol | `LocalExecutor::run` 驱动队列，外层同上 |
    /// | compio | 自己 tick：`Runtime::run` + `poll_with`（与 compio 自己的 `block_on` 同一套循环） |
    ///
    /// compio 那一行不是特例待遇，而是它的队列本就归运行时所有、由运行时自己 tick；
    /// 反过来，tokio / smol 的队列必须由持有者驱动，所以「驱动队列」这一步只能由本方法
    /// 自己做——这正是它不能退化成运行时阻塞原语的原因。
    ///
    /// 一处**如实的差异**：tokio 在 `current_thread` 运行时下，本地队列只能被它自己的
    /// 驱动源推进，所以「调用线程队列里确实有任务要跑」这一情形需要多线程运行时；队列为
    /// 空、只等别处供给时 `current_thread` 也成立（`mptp_cs_demo` 的应用线程即后者）。
    /// 细节与最小复现见 `abs_art-tokio` 的 `LocalScope` 文档。
    ///
    /// # 前提与边界
    ///
    /// - 调用线程必须**就是**本作用域所属的线程（作用域是 `!Send`，编译期即保证）；
    /// - tokio / smol 下 park 期间**本线程不会推进该运行时的 IO driver / 反应器**：
    ///   若本线程同时还负责某个 socket 的 IO，它会停摆。把「驱动连接的线程」与
    ///   「调用本方法的线程」分开（例如 `mptp_cs_demo` 的 `rt_*` 的宿主线程形态）正是
    ///   为这条边界准备的；compio 无此限制；
    /// - 可以嵌套：内层调用会继续驱动同一条队列（tokio / smol 的 `run_until` 允许嵌套，
    ///   compio 本来就只有一个执行器）。
    fn block_on_local<F>(&self, future: F) -> <F as Future>::Output
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
