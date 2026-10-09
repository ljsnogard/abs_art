//! `abs_art` 家族的**后端门面**：全图唯一决定「当前后端」的地方。
//!
//! # 它解决什么
//!
//! `abs_art-bridge` 把「链接哪个后端」（`backend-*`）与「裸名指向谁」
//! （`default-backend-*`）都做成了 feature。但 Cargo feature 是**全图并集且不可撤销**的：
//!
//! - 只要有一条边没关掉 bridge 的 `default`，`default-backend-compio` 就会回来，下游
//!   再也换不了后端（会撞 bridge 的「只能声明一个默认后端」守卫）；
//! - 若反过来要求每条边都写 `default-features = false`，那又逼依赖链上**每个** crate
//!   重复一遍这套知识，漏一条就前功尽弃。
//!
//! 本 crate 把这件事收成**一处**：
//!
//! 1. 它是全家族**唯一**直接依赖 `abs_art-bridge` 的 crate，且带
//!    `default-features = false`；
//! 2. 下游不再认识 `backend-*` / `default-backend-*`，只写 `rt-tokio` / `rt-compio` /
//!    `rt-smol`；
//! 3. 「当前后端」是**全局唯一**的选择：任何一处点亮 `rt-tokio`，全图的 [`Runtime`]
//!    就都是 tokio。**不需要沿依赖边接力传递**，也不需要每层写
//!    `default-features = false`——中间的 crate 只要声明本 crate 为依赖（为了 `use`），
//!    选择与否完全不影响它。
//!
//! bridge 自己的 `default` 保持不动：它单独 `cargo build` 时仍然可用，独立编译这件事
//! 由 bridge 自己负责，与本门面无关。
//!
//! # 它统一转出什么
//!
//! [`Runtime`] / [`LocalScope`]、三个具名别名、[`ScopeHost`] / [`TrRtCurrent`] /
//! [`DefaultRt_`] / [`default_rt_`]，以及 `abs_art` 那批 trait 与能力标签。
//! 下游（`abs_buff_stdio_adapt` / `smux_v1` / `mptp_core` / 业务二进制）只依赖本 crate，
//! 不必再各自维护一份「当前后端」的别名表。
//!
//! # 稳定性的前提
//!
//! 门面的保证建立在一条纪律上：**除本 crate 外谁都不直接依赖 `abs_art-bridge`**。
//! 一旦有别的 crate 直连 bridge 并保留其缺省 feature，`default-backend-compio` 会被
//! 重新拉进来，门面的 `default-features = false` 就白写了。

#![no_std]

pub use abs_art::{
    BLOCK_ON, CLOCK, DELAY, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND, RuntimeTag,
    TrAsyncRuntime, TrBlockOn, TrClock, TrDelay, TrJoinHandle, TrLocalScope, TrMockClock,
    TrSpawnBlocking, TrSpawnSend, TrTime,
};
pub use abs_art_bridge::{LocalScope, Runtime};

// 具名别名：只要对应后端被选中就存在。开关取本 crate 的 `rt-*`，与 bridge 的
// `backend-*` 一一对应（`rt-tokio` 蕴含 `backend-tokio`，其余同理）。
#[cfg(feature = "rt-tokio")]
pub use abs_art_bridge::{TokioJoinHandle, TokioLocalScope, TokioRuntime};

/// 见上（compio 具名别名）。
#[cfg(feature = "rt-compio")]
pub use abs_art_bridge::{CompioJoinHandle, CompioLocalScope, CompioRuntime};

/// 见上（smol 具名别名）。
#[cfg(feature = "rt-smol")]
pub use abs_art_bridge::{SmolJoinHandle, SmolLocalScope, SmolRuntime};

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// 编译期断言：`Runtime` 必须**就是**被选中的那个具名后端。
//
// 这几条同时是「非对称映射」的回归防线：`rt-tokio` / `rt-smol` 用 `default-backend-*`
// 压过 `rt-compio` 的裸 backend；若哪天映射写反了，这里会直接编不过。
// `fn(A) -> B` 的函数指针只在 `A == B` 时成立。
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

/// 选中 tokio 时，裸名就是 tokio（哪怕缺省的 compio 也还链着）。
#[cfg(feature = "rt-tokio")]
const _: fn(TokioRuntime) -> Runtime = |rt| rt;

/// 只选中 smol 时，裸名就是 smol。
#[cfg(feature = "rt-smol")]
const _: fn(SmolRuntime) -> Runtime = |rt| rt;

/// 选中 compio（且没有更高的 override）时，裸名就是 compio。
#[cfg(all(
    feature = "rt-compio",
    not(feature = "rt-tokio"),
    not(feature = "rt-smol")
))]
const _: fn(CompioRuntime) -> Runtime = |rt| rt;

/// **能交出本地作用域**的运行时值。
///
/// `abs_art` 里「取本地作用域」是各后端 `Runtime::local_scope()` 的**固有方法**
/// （不是 trait 入口，见 `abs_art/dev-notes/local-scope-thread-local-20261006-1420.md`
/// 的刻意裁决），泛型代码里写不出 `rt.local_scope()`。本 trait 是那条缺口的补丁：
/// 把固有方法升成一条约束，于是泛型代码可以在只知道「运行时值」的前提下自己取作用域。
///
/// 由于 [`Runtime`] 现在**就是**被选中的那个后端，本 trait 只需要一条实现。
pub trait ScopeHost {
    /// 本运行时值交出的本地作用域类型。
    ///
    /// 要求 `Clone + 'static`：建连路径要把它克隆进各个循环 future。`Clone` 只是
    /// 多一个别名，不是新建队列。
    type Scope: TrLocalScope + Clone + 'static;

    /// 取本线程那条本地队列的别名。
    ///
    /// 语义与各后端的 `Runtime::local_scope()` 完全一致：同一线程上多次调用拿到
    /// **同一条**队列，类型是 `!Send`。
    fn local_scope(&self) -> Self::Scope;
}

/// **能从当前上下文取到一个运行时值**的运行时值。
///
/// 与 [`ScopeHost`] 同一性质、同一理由：「按上下文构造运行时值」是各后端
/// `Runtime::current()` 的固有方法，泛型代码写不出来，因此这里补一条约束。
///
/// # 调用者责任：调用点必须处于后端上下文内
///
/// | 后端 | 上下文要求 | 不在上下文内时 |
/// | --- | --- | --- |
/// | tokio | 处于某个 tokio 运行时上下文内 | `current()` panic（`Handle::current()` 的行为） |
/// | compio | 处于 compio 运行时上下文内 | `current()` panic |
/// | smol | **无**（值是零大小标记） | 不会发生 |
///
/// 跨线程使用连接时「每条线程都自己处于上下文内」这件事**由调用者保证**：debug 构建下
/// [`current_rt`](Self::current_rt) 会先经 [`try_current_rt`](Self::try_current_rt)
/// 给出本 crate 的断言提示，release 构建下由后端的 panic 兜底——两者都属于调用者违约。
pub trait TrRtCurrent: Sized {
    /// 取当前上下文里的运行时值。
    ///
    /// # Panics
    ///
    /// 调用点不在所选后端的运行时上下文内时 panic。
    fn current_rt() -> Self;

    /// 当前是否处于上下文内：`Option::None` = **确定不在**，`Option::Some` = 在。
    ///
    /// 只用于 debug 构建下的提示，不改变任何运行期语义。
    fn try_current_rt() -> Option<Self>;
}

impl ScopeHost for Runtime {
    type Scope = LocalScope;

    fn local_scope(&self) -> Self::Scope {
        Runtime::local_scope(self)
    }
}

impl TrRtCurrent for Runtime {
    fn current_rt() -> Self {
        debug_assert!(
            <Self as TrRtCurrent>::try_current_rt().is_some(),
            "取运行时值时调用点不在所选后端的运行时上下文内：这条前提**由调用者保证**——\
             跨线程复用连接时，每条使用它的线程都必须自己处于后端上下文内\
             （见 `TrRtCurrent` 文档）。",
        );
        Runtime::current()
    }

    fn try_current_rt() -> Option<Self> {
        Runtime::try_current()
    }
}

/// **当前装配的运行时值类型**：就是被选中的那个后端（[`Runtime`]）。
pub type DefaultRt_ = Runtime;

/// 编译期断言：当前后端必须满足「计时 + 可克隆 + 能交出作用域」这组约束。
const _: fn() = || {
    fn assert_rt_<R: TrTime + Clone + 'static + ScopeHost>() {}
    assert_rt_::<DefaultRt_>();
};

/// 构造**当前装配的运行时值**：等价于所选后端的 `current()`。
///
/// # Panics
///
/// 调用点不在所选后端的运行时上下文内时 panic（文案由各后端给出）。这是「自动取运行时值」
/// 那条路的既定契约：不在上下文内时，请改走显式传入运行时值的入口。
pub fn default_rt_() -> DefaultRt_ {
    <DefaultRt_ as TrRtCurrent>::current_rt()
}

/// **虚拟时间**的运行时值：把作用域请求委托给被装饰的运行时值。
///
/// `abs_art_mock_clock::ManualTime<R, C>` 只把**时间**换成手动时钟（`TrDelay` /
/// `TrClock` / `TrTime`），其余能力（含「哪条本地队列」）委托给 `R`。连接要自己取
/// 作用域，因此这里把这条请求也一并委托下去：**时间可以是虚拟的，队列仍是那个后端
/// 本来的那条**。
///
/// # 为什么写在这里
///
/// [`ScopeHost`] 是本 crate 的 trait，而 `ManualTime` 是外部类型；对「外部 trait +
/// 外部类型」写 impl 会触发 `E0117`。因此这条实现只能留在这里，并由 `mock-clock`
/// feature 拉进 `abs_art-mock_clock`（生产依赖图里不出现）。
#[cfg(feature = "mock-clock")]
impl<R, C> ScopeHost for abs_art_mock_clock::ManualTime<R, C>
where
    R: ScopeHost,
    C: abs_art_mock_clock::ManualClockApi,
{
    type Scope = R::Scope;

    fn local_scope(&self) -> Self::Scope {
        ScopeHost::local_scope(self.inner())
    }
}
