//! `abs_art` 的 smol 组合 crate。
//!
//! 提供五个功能（各自为 feature 开关）：
//!
//! - `block_on`：阻塞等待一个 future 完成（**不**驱动本地队列，见下）；
//! - `delay`：睡眠 / 延迟执行，以及计时能力（[`TrClock`] / [`TrTime`]）；
//! - `spawn_send`：投递任务到进程级全局工作队列；
//! - `local_scope`：本地的 `!Send` 队列（[`LocalScope`]，投递 + 驱动）；
//! - `spawn_blocking`：投递阻塞函数到进程级阻塞线程池。
//!
//! 所有实现都基于基础 crate [`abs_art`] 中的 trait。
//!
//! # 运行时值是**零大小的标记**——smol 什么都钉不住
//!
//! [`Runtime`] 不持有任何东西：smol 既没有「环境运行时句柄」可捕获（它没有 tokio
//! 那样的 `Handle::current()`），全局执行器与阻塞线程池又都是**进程级单例**，任何值
//! 都持不了（理由见 `spawn_send` 模块文档）。因此
//!
//! - [`Runtime`] 是 **ZST**（`size_of == 0`）且 `Send + Sync`——可以放进 `static`、
//!   可以跨线程传，但也**不带任何队列**；
//! - 它存在的意义是**统一调用形状**（`rt.spawn(..)` / `rt.delay(..)` / `rt.now()`）
//!   与**能力位声明**（`Runtime<CAPS>` 决定这个类型实现了哪些能力 trait）。
//!
//! **本地队列是另一件事**：`!Send` 任务要投到「哪条队列、由谁驱动」，答案由
//! [`Runtime::local_scope()`] 交出的 [`LocalScope`] 回答。那条队列是**调用方创建、
//! 调用方驱动的独立对象**，与运行时值互不归属：
//!
//! - `spawn_local` / `run_until` / `block_on`（阻塞驱动）都在 [`LocalScope`] 上；
//! - 取得作用域要先把「我要用本地投递」写在类型上：`Runtime<CAPS>::local_scope()`
//!   要求 `CAPS` 含 [`SPAWN_LOCAL`]；
//! - [`Runtime::block_on`](abs_art::TrBlockOn::block_on) **只等待、不驱动**任何本地
//!   队列。
//!
//! ## `spawn` 是唯一的例外，且这条例外是诚实的
//!
//! smol 的全局执行器（`smol::spawn` 内部用 `OnceCell<Executor>` 懒初始化，交给后台
//! 线程永久驱动，见 smol 2.0.2 的 `src/spawn.rs`）**就是进程级多线程执行器**：它
//! 一直在跑、不能被替换、也不能被 drop。所以本 crate 的选择是：`TrSpawnSend::spawn`
//! 如实转发到它，并把「钉不住」写进文档，而不是伪造一个「每值一份」的假象。
//! 可观测后果（`abs_art-smol` 的测试里钉住了前两条）：
//!
//! 1. **值被 drop 不会停止已投递的全局任务**：任务归进程级执行器所有；
//! 2. **同一个进程里的两个运行时值的 `spawn` 共享同一条全局队列**：它们之间没有
//!    队列边界，`Runtime` 值的数量不改变全局并发度；
//! 3. **全局并发度由环境变量 `SMOL_THREADS` 决定**（缺省 1 个后台线程），它是进程
//!    级配置，不是本值的能力。
//!
//! 需要「队列随作用域走」的隔离语义时用 [`TrLocalScope::spawn_local`]——那才是
//! [`LocalScope`] 真正拥有的队列。
//!
//! ## 值钉住了什么（如实表）
//!
//! | 能力 | 载体 | [`Runtime`] 值能否钉住 |
//! | --- | --- | --- |
//! | `spawn` | `smol::spawn` 的进程级全局执行器（`OnceCell`） | **不能**（见上） |
//! | `spawn_blocking` | `blocking` 的进程级线程池 | **不能**（进程级资源） |
//! | `delay` / `now` / `interval` | async-io 的**进程级反应器**；值提供 `Instant` 基准与调用形状 | **部分**：反应器进程级，但 `TrTime: TrDelay + TrClock` 让「睡在哪个基准、读哪个时刻」由同一个值回答 |
//! | `spawn_local` / `run_until` / 阻塞驱动 | **不在值上**：[`LocalScope`] 持有的 `Rc<LocalExecutor<'static>>` | 值只负责**交出**作用域；队列归作用域，且是 `!Send` |
//!
//! 这是本 crate 与 `abs_art-tokio` 的形状差异：tokio 的值是一个 `Send + Sync` 的
//! `Handle`（钉得住全局 spawn），smol 的值什么也钉不住（ZST）。
//!
//! # 两种用法
//!
//! ## 具体用法（二进制层）
//!
//! ```
//! use abs_art::TrBlockOn;
//!
//! let rt = abs_art_smol::current();
//! let out = rt.block_on(async { 42 });
//! assert_eq!(out, 42);
//! ```
//!
//! ## 能力位用法（业务库层，编译期能力检查）
//!
//! 通过 const 泛型声明所需能力；请求了未声明（或本后端不支持）的能力会在
//! 编译期报错：
//!
//! ```
//! use abs_art::{BLOCK_ON, SPAWN_SEND, TrBlockOn, TrSpawnSend};
//! use abs_art_smol::Runtime;
//!
//! // 只声明 block_on + spawn_send 两种能力
//! let rt = Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current();
//! let handle = rt.spawn(async { 2 });
//! assert_eq!(rt.block_on(handle).unwrap(), 2);
//! ```
//!
//! ```compile_fail
//! use abs_art::{BLOCK_ON, TrSpawnSend};
//! use abs_art_smol::Runtime;
//!
//! // 只声明了 block_on 能力，spawn（spawn_send）不可用 → 编译错误（Tag 严格模式）
//! let rt = Runtime::<{ BLOCK_ON }>::current();
//! let _ = rt.spawn(async { 1 });
//! ```
//!
//! ## 本地投递（`!Send` 任务）
//!
//! 本地队列是一个独立的作用域值，由运行时值交出来：
//!
//! ```
//! use abs_art::{SPAWN_LOCAL, TrLocalScope};
//! use abs_art_smol::Runtime;
//!
//! // `local_scope()` 要求 CAPS 含 SPAWN_LOCAL（这里显式写出来）
//! let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();
//! let out = scope.block_on(async {
//!     let rc = std::rc::Rc::new(6u32); // !Send：只有本地队列能承载
//!     scope.spawn_local(async move { *rc * 7 }).await.unwrap()
//! });
//! assert_eq!(out, 42);
//! ```

#![no_std]

extern crate alloc;

// smol 2.x 无条件依赖 `std`（`smol::spawn` 起后台线程、`blocking` 用线程池），
// 本 crate 沿用 `no_std` 写法只是「不注入 std prelude」。但 `TrClock::Instant`
// 需要写出 `std::time::Instant`——那正是 async-io 计时器所用的时间基准
// （`async_io::Timer::after` 的到期时刻就是按 `std::time::Instant` 算的），
// 因此必须显式引入 `std`。它**不新增**任何依赖。
extern crate std;

// `RuntimeTag` 与基础 crate 的其它公开项一起重导出（保持本 crate 原有的公开面）。
pub use abs_art::{
    BLOCK_ON, DELAY, Elapsed, RuntimeTag, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, Timeout,
    TrAsyncRuntime, TrBlockOn, TrClock, TrDelay, TrInterval, TrJoinHandle, TrLocalScope,
    TrSpawnBlocking, TrSpawnSend, TrTime, UnitFuture,
};

/// 本后端的**完整能力集**。
///
/// smol 有真正的跨线程全局工作队列（`spawn` 投到进程级全局执行器、`spawn_blocking`
/// 投到进程级阻塞线程池），五种能力一个不缺，因此本常量与 [`abs_art::FULL`]
/// **数值相同**。与之相对，`abs_art-compio` 的 `FULL` 不含 `SPAWN_SEND`（它没有那种
/// 队列）——所以「完整」是**按后端**回答的问题，本常量是本后端的答案。
///
/// 与 [`abs_art::FULL`] 的分工：
///
/// - [`abs_art::FULL`] 是**位集合意义上**的「全部能力位」（基础 crate 给出的全集）；
/// - 本常量是**本后端实现得了的**那一部分。
///
/// 两者当前相等只是「smol 每种能力都有」的结果，不是同义反复；某个后端补齐不了
/// 某个位时，差异体现在各后端的 `FULL` 上。本 crate 的 [`Runtime`] 默认类型参数
/// 用的就是本常量。
pub const FULL: usize = abs_art::FULL;

/// smol 组合运行时**值**。
///
/// 对应基础 crate 中的 [`RuntimeTag::Smol`]。它是**零大小的标记**（`size_of == 0`）：
/// smol 没有可捕获的环境运行时，全局执行器与阻塞线程池又都是进程级单例，因此本值
/// **不持有任何状态**——本地队列也不在它里面（那是线程独占资源，见 [`LocalScope`]）。
///
/// 类型参数 `CAPS` 是能力位掩码（见 [`abs_art::caps`]）：默认 [`FULL`]（全功能），
/// 也可以写成 `Runtime<{ BLOCK_ON | SPAWN_SEND }>` 只声明部分能力。掩码决定这个
/// **类型**实现了哪些能力 trait、以及能不能经 `local_scope()` 取得作用域。
///
/// # 构造
///
/// 见 [`Runtime::current`]：smol 没有「环境运行时」可捕获，因此构造**没有先决
/// 条件**，任何线程都能直接构造（这一点与 `abs_art-tokio` 的 `Handle::current()`
/// 不同）。需要默认能力位时用 crate 级自由函数 [`current`]。
///
/// # `Send + Sync`
///
/// 本值是 ZST、不含队列，因此是 `Send + Sync` 的：可以跨线程传、可以放进 `static`
/// （`abs_art-tokio` 的值同样是 `Send + Sync`，但它装着一个 `Handle`；本值什么都不装）。
/// 线程独占的是 [`LocalScope`]——它的 `Rc<LocalExecutor<'static>>` 是 `!Send`。
///
/// # 克隆
///
/// `Clone` 只是复制这个零大小的标记（同一个类型、同一个能力集），不涉及任何队列：
/// 要「同一条队列的第二个把手」，克隆 [`LocalScope`]；要「另一条队列」，再调一次
/// [`Runtime::local_scope()`]。
pub struct Runtime<const CAPS: usize = FULL>;

/// 构造**全能力**（`Runtime<FULL>`）运行时值。
///
/// 这是最常用的构造入口：类型参数 `CAPS` 直接取默认值 [`FULL`]，因此在表达式
/// 位置也**不需要类型标注**（`Runtime::current()` 在表达式位置会因 `CAPS` 无法
/// 推断而报 `E0284`，自由函数不会）。需要显式声明能力时用
/// [`Runtime::current`] 的 turbofish 形式。
///
/// # Panics
///
/// 不 panic：smol 没有「环境运行时」这一前提。
///
/// # Examples
///
/// ```
/// let rt = abs_art_smol::current();
/// assert_eq!(rt.tag(), abs_art::RuntimeTag::Smol);
/// ```
pub fn current() -> Runtime {
    Runtime::current()
}

impl<const CAPS: usize> Runtime<CAPS> {
    /// 构造运行时值。
    ///
    /// smol 没有「环境运行时句柄」可捕获：全局执行器是进程级单例（见 crate 文档）。
    /// 因此本函数没有先决条件，也不会捕获任何环境状态——它交出一个零大小的标记，
    /// 全局执行器与任何队列都不受影响。
    ///
    /// # Panics
    ///
    /// 不 panic（与 `abs_art-tokio` 的 `Runtime::current` 不同，后者要求调用点处于
    /// tokio 运行时上下文内）。
    ///
    /// # Examples
    ///
    /// 注意表达式位置必须写全类型（默认的 `CAPS = FULL` **不参与**表达式位置的
    /// 推断，`Runtime::current()` 会因 const 泛型无法推断而报 `E0284`）；需要默认
    /// 能力位时用 crate 级自由函数 [`current`]。
    ///
    /// ```
    /// use abs_art_smol::Runtime;
    ///
    /// let value = Runtime::<{ abs_art::FULL }>::current();
    /// assert_eq!(value.tag(), abs_art::RuntimeTag::Smol);
    /// ```
    pub fn current() -> Self {
        Self
    }

    /// 复制这个标记，并在 CAPS 上「换标签」。
    ///
    /// 与 `Clone` 等价。因为本值是 ZST，这里**不涉及**任何队列或运行时资源
    /// （tokio 侧这里复制的是运行时句柄，smol 侧没有可复制的句柄）。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art_smol::Runtime;
    ///
    /// let a = Runtime::<{ abs_art::FULL }>::current();
    /// let b = a.retag::<{ abs_art::FULL }>();
    /// assert_eq!(b.tag(), abs_art::RuntimeTag::Smol);
    /// ```
    pub fn retag<const OTHER: usize>(&self) -> Runtime<OTHER> {
        Runtime::<OTHER>
    }

    /// 本值对应的抽象运行时标签。
    pub fn tag(&self) -> RuntimeTag {
        RuntimeTag::Smol
    }
}

#[cfg(feature = "local_scope")]
impl<const CAPS: usize> Runtime<CAPS>
where
    [(); CAPS]: abs_art::HasSpawnLocal,
{
    /// 交出**一条新的**本地队列——一个线程独占的 [`LocalScope`]。
    ///
    /// # 为什么要求能力位
    ///
    /// 这是取得本地投递能力的**唯一入口**，而它要求 `CAPS` 含
    /// [`SPAWN_LOCAL`]：想拿到作用域，就得先把「我要用本地
    /// 投递」这件事写在类型上。声明位的价值是「**必须写下来**」，不是「写不下来
    /// 就用不了」。
    ///
    /// # 每次调用得到的是**新队列**
    ///
    /// smol 没有可捕获的运行时，队列完全由本函数新建（`Rc<LocalExecutor<'static>>`）；
    /// 也就是说本方法**不读取 `self`**（`&self` 只是为了三后端调用形状一致）。要
    /// 「同一条队列的第二个把手」，克隆交出的作用域（[`LocalScope`] 的 `Clone` 共享
    /// 同一个 `Rc`）；要两条独立队列，调两次本方法。
    ///
    /// # 线程独占
    ///
    /// 队列是**调用方创建、调用方驱动**的独立对象，且 `Rc` 使它 `!Send`：作用域只能
    /// 在**这条**线程上被驱动。需要在别的线程上投递本地任务时，在那边另取一个作用域
    /// （本方法无先决条件，任何线程都能调）。
    ///
    /// # Examples
    ///
    /// ```
    /// use abs_art::{SPAWN_LOCAL, TrLocalScope};
    /// use abs_art_smol::Runtime;
    ///
    /// let scope = Runtime::<{ SPAWN_LOCAL }>::current().local_scope();
    /// let out = scope.block_on(async {
    ///     scope.spawn_local(async { 7u32 }).await.unwrap()
    /// });
    /// assert_eq!(out, 7);
    /// ```
    pub fn local_scope(&self) -> LocalScope {
        LocalScope::with_executor()
    }
}

impl<const CAPS: usize> Clone for Runtime<CAPS> {
    /// 复制零大小的标记：不涉及任何队列与运行时资源。
    fn clone(&self) -> Self {
        Self
    }
}

impl<const CAPS: usize> core::fmt::Debug for Runtime<CAPS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("abs_art_smol::Runtime")
            .field("caps", &CAPS)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "join_handle")]
pub mod join_handle;

#[cfg(feature = "join_handle")]
pub use join_handle::{JoinError, JoinHandle};

#[cfg(feature = "block_on")]
mod block_on;

#[cfg(feature = "delay")]
pub mod delay;

#[cfg(feature = "delay")]
pub mod time;

#[cfg(feature = "spawn_send")]
mod spawn_send;

#[cfg(feature = "local_scope")]
pub mod local_scope;

#[cfg(feature = "local_scope")]
pub use local_scope::LocalScope;

#[cfg(feature = "spawn_blocking")]
mod spawn_blocking;

#[cfg(test)]
mod tests {
    //! 针对 smol 后端**运行时值形状**的单元测试。

    use crate::{FULL, Runtime};

    /// 目的：验证运行时**值**是零大小的标记，且是 `Send + Sync`（因此能放进
    /// `static` 跨线程共享）——本地队列不在它里面（也不在 smol 的任何值里）。
    ///
    /// 手段：在编译期用 `assert_send_sync::<Runtime<{ FULL }>>()` 断言自动 trait；
    /// 在 `static` 位置 const 构造一个全能力值（只有 `Sync` 类型才能进 `static`）；
    /// 再在运行期用 `core::mem::size_of` 断言该类型大小为 0。
    ///
    /// 判定：编译通过（前两条）且 `size_of::<Runtime<FULL>>() == 0`（第三条）。
    /// 若有人把 `Rc<LocalExecutor>` 塞回值里，三条都会失败：`Rc` 会让类型 `!Send`，
    /// 进不了 `static`，也不再是 ZST。
    #[test]
    fn runtime_value_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime<{ FULL }>>();

        static SHARED_: Runtime<{ FULL }> = Runtime::<{ FULL }>;
        assert_eq!(SHARED_.tag(), crate::RuntimeTag::Smol);

        assert_eq!(
            core::mem::size_of::<Runtime<{ FULL }>>(),
            0,
            "运行时值必须是 ZST：smol 侧它不持有任何状态"
        );
    }

    /// 目的：验证值的形状不随 feature / 能力位漂移——一个能力位都没声明
    /// （`Runtime<0>`）时，它仍是可构造、`Send + Sync`、零大小的标记。
    ///
    /// 手段：编译期断言 `Runtime<0>: Send + Sync`，运行期断言其大小为 0。
    ///
    /// 判定：编译通过且断言成立；若值的定义依赖某个 feature（例如关闭 `local_scope`
    /// 时换成另一种结构），本用例会暴露形状漂移。
    #[test]
    fn runtime_value_shape_does_not_depend_on_features() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime<0>>();
        assert_eq!(core::mem::size_of::<Runtime<0>>(), 0);
    }

    /// 目的：验证本后端的 `FULL` 是**本后端的完整能力集**——数值与
    /// [`abs_art::FULL`] 相同，且确实覆盖全部六种能力。
    ///
    /// 手段：运行期断言 `FULL == abs_art::FULL` 且 `FULL` 含 `CLOCK`
    /// （`FULL & CLOCK != 0`）；编译期把 `[(); FULL]` 依次传给六个标记 trait 的断言函数。
    ///
    /// 判定：断言成立且编译通过即为通过。smol 有进程级全局执行器与阻塞线程池，
    /// 五种能力一个不缺，因此这里与基础 crate 的全集相等；若将来某位不再支持，
    /// 本用例会先失败，迫使各后端 `FULL` 与文档同步。
    #[test]
    fn full_is_this_backend_complete_capability_set() {
        use abs_art::{
            HasBlockOn, HasClock, HasDelay, HasSpawnBlocking, HasSpawnLocal, HasSpawnSend,
        };

        fn assert_block_on<T: HasBlockOn>() {}
        fn assert_delay<T: HasDelay>() {}
        fn assert_spawn_send<T: HasSpawnSend>() {}
        fn assert_spawn_local<T: HasSpawnLocal>() {}
        fn assert_spawn_blocking<T: HasSpawnBlocking>() {}
        fn assert_clock<T: HasClock>() {}

        assert_eq!(
            FULL,
            abs_art::FULL,
            "smol 的完整能力集应与基础 crate 的全集同值"
        );
        assert_ne!(FULL & abs_art::CLOCK, 0, "本后端的完整能力集必须含 CLOCK");

        assert_block_on::<[(); FULL]>();
        assert_delay::<[(); FULL]>();
        assert_spawn_send::<[(); FULL]>();
        assert_spawn_local::<[(); FULL]>();
        assert_spawn_blocking::<[(); FULL]>();
        assert_clock::<[(); FULL]>();
    }
}
