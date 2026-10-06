//! `abs_art-demo`：演示「业务库零泛型穿透 + 集成方通过 Cargo.toml 选后端」。
//!
//! - [`lib.rs`](crate)（业务库）：只依赖 [`abs_art_bridge`]（当前后端实例，
//!   见下），用能力标签 `Runtime::<{ BLOCK_ON | SPAWN_SEND }>` 声明所需能力；
//!   v0.4 起运行时是**值**，因此业务函数接收 `&Runtime<CAPS>` 并在这个值上调用
//!   能力方法，而不再靠「类型即能力」直接调关联函数；
//! - `main.rs`（二进制）：负责**创建运行时 → 构造运行时值 → 传给业务库**——
//!   它是唯一允许感知后端的地方（创建哪个运行时的代码必须与所选后端一致）。
//!
//! # 后端选择（本 crate 的 features）
//!
//! `Cargo.toml` 里 `demo-tokio` 与 `demo-compio` **互斥**，一次构建只能启用
//! 一个：
//!
//! ```text
//! cargo run -p abs_art-demo                                        # tokio 组（默认）
//! cargo run -p abs_art-demo --no-default-features --features demo-compio   # compio 组
//! ```
//!
//! 本 crate 把同一个 `abs_art-bridge` 以两个 backend 实例化（重命名依赖
//! `bridge_tokio` / `bridge_compio`），`lib.rs` 用 `pub use ... as
//! abs_art_bridge` 把「当前后端实例」统一暴露为 `abs_art_bridge`——业务代码
//! 的写法与后端无关。
//!
//! # 运行时是值（v0.4）
//!
//! 业务函数的形状从「类型参数 + 关联函数」变成「**值参数 + 方法调用**」：
//!
//! ```text
//! v0.3:  <CapRt as TrBlockOn>::block_on(async { <CapRt as TrSpawnSend>::spawn(..) })
//! v0.4:  rt.block_on(async { rt.spawn(..) })
//! ```
//!
//! 关键收益是**环境与能力绑定在同一个值上**：`spawn` 打在「这个值抓住的运行时」
//! 上，`spawn_local` / `run_until` 打在「这个值持有的本地队列」上，
//! `delay` / `now` 来自「这个值的计时源」。本地投递不再是独立的 `LocalScope`
//! 值，[`TrLocalScope`] 直接实现在 `Runtime<CAPS>` 上——**运行时值自己就是本地
//! 作用域**。
//!
//! 构造入口（集成方使用）：
//!
//! - 全能力：`abs_art_bridge::current()`（crate 级自由函数，无需类型参数）；
//! - 显式能力：`Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current()`（表达式位置
//!   必须带 turbofish 或经类型别名，否则 const 参数无法推断）。
//!
//! # Examples（每种 cap 一个 smoke test，按后端分组）
//!
//! `examples/tokio_demo/` 与 `examples/compio_demo/` 各 7 个 demo，一一对应，
//! 用**同一种 cap 组合**验证 `abs_art` / `abs_art-bridge` 的一个设计意图，
//! 文档注释里写明「要验证什么 / 可以做到什么 / 不能做到什么」：
//!
//! - `cap_block_on.rs`：`BLOCK_ON` —— 最小能力 + `TrBlockOn` 放松 `'static`
//!   后可以驱动借用栈数据的 future / 返回借用引用；
//! - `cap_spawn_send.rs`：`BLOCK_ON | SPAWN_SEND` —— 跨线程（compio 为
//!   线程本地交错）spawn、`TrJoinHandle` 句柄抽象、JoinErr 传播；
//! - `cap_spawn_local.rs`：本地投递**值** —— `!Send` 的 `Rc` 任务经
//!   `rt.spawn_local(..)` 投递、经 `rt.run_until(..)` 驱动（队列归运行时值所有）；
//! - `cap_delay.rs`：`DELAY` —— 时间驱动与 time driver 前提；
//! - `cap_spawn_blocking.rs`：`BLOCK_ON | SPAWN_BLOCKING` —— 阻塞线程池与
//!   异步侧共存；
//! - `cap_full.rs`：`FULL` —— 五个能力位（含 `SPAWN_LOCAL` 声明位）+ 本地投递
//!   协同 + 后端自省（`rt.tag()` / `rt.about()`）；
//! - `cap_zero.rs`：`0` —— 零能力边界：`Runtime<0>` 只是值类型，任何能力调用
//!   都是编译错误。
//!
//! 「不能做到什么」的编译期负向演示集中在
//! [`strict_mode_check`]（`compile_fail` 文档测试，`cargo test --doc` 验证，
//! 两种后端下均生效）。
//!
//! # 后端实例与互斥守卫
//!
//! 当前后端由 `demo-*` feature 决定，本 crate 对外统一暴露为
//! [`abs_art_bridge`]（模块），并直接把能力项再导出到 crate 根，供
//! doctest / 下游以 `abs_art_demo::*` 引用，与具体后端无关。

// 后端实例：demo-tokio / demo-compio 二选一（见 Cargo.toml 的 features）。
// 两个分支都是 `pub use` 外部 crate 的重命名导入，把「当前后端的 bridge」
// 统一暴露为 abs_art_bridge。
#[cfg(feature = "demo-tokio")]
pub use bridge_tokio as abs_art_bridge;
#[cfg(feature = "demo-compio")]
pub use bridge_compio as abs_art_bridge;

// 互斥守卫：两个演示组同时启用 → 响亮报错（而不是静默选一个）。
// bridge 内部对「backend 只能启用一个」还有一道 compile_error，这里先拦住。
#[cfg(all(feature = "demo-tokio", feature = "demo-compio"))]
compile_error!("abs_art-demo：demo-tokio 与 demo-compio 互斥，一次构建只能启用一个");

// 至少要启用一个演示组，否则上面的 abs_art_bridge 别名不存在。
#[cfg(not(any(feature = "demo-tokio", feature = "demo-compio")))]
compile_error!("abs_art-demo：必须启用 demo-tokio 或 demo-compio 之一（默认 demo-tokio）");

/// 能力项再导出：业务库对外暴露与后端无关的能力入口。
///
/// 这样 doctest / 下游代码可以统一写 `abs_art_demo::BLOCK_ON` 而不必关心
/// 当前启用的是哪个后端实例。
pub use abs_art_bridge::{
    BLOCK_ON, DELAY, FULL, Runtime, RuntimeTag, SPAWN_BLOCKING, SPAWN_LOCAL,
    SPAWN_SEND, TrAsyncRuntime, TrBlockOn, TrClock, TrDelay, TrJoinHandle,
    TrLocalScope, TrSpawnBlocking, TrSpawnSend, TrTime, current,
};

/// 业务库声明的能力组合：只需要 `block_on` + `spawn_send` 两种能力。
///
/// 请求了未声明（或当前后端不支持）的能力会在编译期报错（Tag 严格模式）。
pub type CapRt = Runtime<{ BLOCK_ON | SPAWN_SEND }>;

/// 最小能力声明：只需要 `block_on` 一种能力。
///
/// 对应 `examples/{tokio,compio}_demo/cap_block_on.rs` 的 smoke test；这里
/// 作为业务库函数，展示 `TrBlockOn` 放松 `'static` 约束（提交 `19a6525`）
/// 带来的实际收益——两个后端都支持（tokio 的 `Handle::block_on` 与 compio
/// 的 `Runtime::block_on` 均接受非 `'static` 的 future）。
pub type BlockOnRt = Runtime<{ BLOCK_ON }>;

/// 本地投递所需的能力声明：`SPAWN_LOCAL` 位 + `block_on`。
///
/// 注意 `SPAWN_LOCAL` 只负责**声明**：真正能不能调 `spawn_local` 由**值**
/// 决定——[`TrLocalScope`] 直接实现在持有本地队列的 `Runtime<CAPS>` 上，
/// 而该 impl 由本位门控（见 [`strict_mode_check::spawn_local_requires_declaration`]）。
pub type LocalRt = Runtime<{ BLOCK_ON | SPAWN_LOCAL }>;

/// 示例业务函数：在这个运行时值上 spawn 一个任务计算 `x * 2`，再 `block_on`
/// 等待结果。
///
/// 该函数**没有任何泛型参数**——运行时由**值参数** `rt` 提供，能力边界通过
/// [`CapRt`] 的类型参数静态声明，编译期完成校验，运行期零开销。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{double_via_runtime, CapRt};
///
/// // 运行时值必须由处于后端上下文的一方构造（此处为 `no_run`，不执行）
/// let rt = CapRt::current();
/// assert_eq!(double_via_runtime(&rt, 21), 42);
/// ```
pub fn double_via_runtime(rt: &CapRt, x: i32) -> i32 {
    rt.block_on(async move {
        let handle = rt.spawn(async move { x * 2 });
        handle.await.unwrap()
    })
}

/// 示例业务函数：在这个运行时值上 `block_on` 一个**借用栈上数据**的 future
/// （非 `'static`）。
///
/// 旧约束（`F: Future + 'static` 且 `F::Output: 'static`）下，`async` 块借用
/// 局部 `data` 无法通过编译；上一提交把 [`TrBlockOn`] 的约束放松为
/// `F: Future` 后，这类「只在本栈帧内同步等一个 async 结果」的代码得以成立。
///
/// 注意：本函数只通过 [`BlockOnRt`] 的类型参数声明能力、并在传入的**值**上
/// 调用；「必须处于运行时上下文内」是后端契约（tokio 需要 `Handle::current()`、
/// compio 需要 `with_current`），由集成方保证。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{sum_stack_data, BlockOnRt};
///
/// let rt = BlockOnRt::current();
/// assert_eq!(sum_stack_data(&rt), 10);
/// ```
pub fn sum_stack_data(rt: &BlockOnRt) -> usize {
    let data = [1usize, 2, 3, 4];
    rt.block_on(async { data.iter().sum() })
}

/// 纯泛型业务函数：只写**一个** `Rt: TrSpawnSend + TrBlockOn` 约束，就 spawn
/// 两种不同的 future（其中第二个是无法命名的 `async {}` 块），再 `block_on`
/// 等待结果。
///
/// 这是 v0.3 把自由参数 `F` 从 trait 泛型列表移到 [`TrSpawnSend::spawn`]
/// 方法级泛型后获得的能力：v0.2 需要为每个 future 各写一条约束
/// （`Rt: TrSpawnSend<F1> + TrSpawnSend<F2>`），而函数内部 `async` 块的类型
/// 外部**无法命名**，`F2` 那条约束根本写不出来。本函数不感知任何后端，
/// `Rt` 由调用方（集成方）以**值**的形式给出。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{generic_two_tasks, CapRt};
///
/// let rt = CapRt::current();
/// assert_eq!(generic_two_tasks(&rt, 21), 63);
/// ```
pub fn generic_two_tasks<Rt>(rt: &Rt, x: i32) -> i32
where
    Rt: TrSpawnSend + TrBlockOn,
{
    rt.block_on(async move {
        let a = rt.spawn(async move { x }).await.unwrap();
        // 第二个 future 是匿名 async 块：单一 `Rt: TrSpawnSend` 约束即可覆盖
        let b = rt.spawn(async move { a * 2 }).await.unwrap();
        a + b
    })
}

/// 本地投递示例：**运行时值自己就是本地作用域**——投递点与驱动点都在 `rt` 上。
///
/// 这是 v0.4 相对 v0.3 的形状变化：v0.3 需要先拿一个独立的 `LocalScope` 值
/// （它可能被复制、被传递、被指向别处），再 `scope.spawn_local(..)` 并
/// `scope.run_until(..)`；v0.4 把本地队列并回运行时值，因此：
///
/// - [`TrLocalScope::spawn_local`] 投到**这个值**持有的队列；
/// - [`TrLocalScope::run_until`] 驱动**这个值**持有的队列。
///
/// 本函数是 `async fn`：`run_until` 返回的 future 需要放在「已处于该后端运行时
/// 上下文」的位置 await（由集成方的最外层 `rt.block_on(..)` 提供，见
/// `src/main.rs` 与各 example 的 `main`）。`Rc` 是 `!Send`，只有本地队列能承载。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{local_rc_double, LocalRt};
///
/// # fn drive<T>(f: impl std::future::Future<Output = T>) -> T {
/// #     unimplemented!()
/// # }
/// let rt = LocalRt::current();
/// assert_eq!(drive(local_rc_double(&rt, 21)), 42);
/// ```
pub async fn local_rc_double(rt: &LocalRt, x: i32) -> i32 {
    rt.run_until(async move {
        // Rc 是 !Send：只有「线程本地」的队列能承载这样的任务。
        let rc = std::rc::Rc::new(x);
        rt.spawn_local(async move { *rc * 2 }).await.unwrap()
    })
    .await
}

/// 目的：验证业务函数在真实 tokio 后端上运行正常（值语义）。
///
/// 实施策略：由本 crate 选定的演示组（`demo-tokio`）创建 tokio 运行时，
/// 在运行时上下文内**构造运行时值**，再把它交给
/// [`double_via_runtime`] / [`sum_stack_data`] / [`local_rc_double`]。
///
/// 通过依据：`double_via_runtime(&rt, 21) == 42`；`sum_stack_data(&rt) == 10`；
/// `generic_two_tasks(&rt, 21) == 63`；`local_rc_double(&rt, 21) == 42`。
#[cfg(all(test, feature = "demo-tokio"))]
mod tests_tokio {
    use super::*;

    /// 目的：验证最基础的「spawn + block_on」业务函数在 tokio 值上可用。
    ///
    /// 实施策略：创建多线程 tokio 运行时，在其 `block_on` 上下文内构造
    /// [`CapRt`] 值（`current()` 需要环境上下文），再调用
    /// [`double_via_runtime`]。
    ///
    /// 通过依据：返回 21 * 2 == 42 且无 panic；若运行时值没有被正确传递到业务
    /// 函数（例如误用类型级关联函数），本测试将无法编译。
    #[test]
    fn business_fn_works() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = CapRt::current();
            double_via_runtime(&value, 21)
        });
        assert_eq!(out, 42);
    }

    /// 目的：验证放松 `'static` 后的 `TrBlockOn` 在业务库层面可用。
    ///
    /// 实施策略：在 tokio 运行时上下文内构造 [`BlockOnRt`] 值，调用
    /// [`sum_stack_data`]（内部 `block_on` 一个借用局部数据的 future）。
    ///
    /// 通过依据：返回 1 + 2 + 3 + 4 == 10；若 `'static` 约束未放松，
    /// 本测试将无法编译。
    #[test]
    fn borrow_block_on_works() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async {
            let value = BlockOnRt::current();
            sum_stack_data(&value)
        });
        assert_eq!(out, 10);
    }

    /// 目的：验证 v0.3 的单一 `Rt: TrSpawnSend` 约束能覆盖多个 future
    /// （含无法命名的 `async` 块）——即把 `F` 移到方法级泛型后真正拿到的收益。
    ///
    /// 实施策略：在 tokio 多线程运行时上下文内调用泛型函数
    /// [`generic_two_tasks`]，传入只声明 `BLOCK_ON | SPAWN_SEND` 的 [`CapRt`]
    /// 值；该函数内部仅凭一条 `Rt: TrSpawnSend` 约束 spawn 两个不同的 future
    /// （第二个是匿名 `async` 块）。
    ///
    /// 通过依据：返回 `21 + 21 * 2 == 63`；若 trait 退回 v0.2 的
    /// `TrSpawnSend<F>` 形状（无法为匿名 future 命名约束），本测试将无法编译。
    #[test]
    fn generic_bound_covers_multiple_futures() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async {
            let value = CapRt::current();
            generic_two_tasks(&value, 21)
        });
        assert_eq!(out, 63);
    }

    /// 目的：验证**本地投递**已并入运行时值——`rt.spawn_local` 投递 `!Send`
    /// 任务、`rt.run_until` 驱动同一个值的队列。
    ///
    /// 实施策略：在 current_thread tokio 运行时上下文内构造 [`LocalRt`] 值，
    /// 调用 [`local_rc_double`]（内部 `Rc` 任务 + `run_until`）。
    ///
    /// 通过依据：取回 21 * 2 == 42；若本地队列没有被这个值驱动，
    /// `run_until` 将永久挂起。
    #[test]
    fn local_delivery_works() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async {
            let value = LocalRt::current();
            local_rc_double(&value, 21).await
        });
        assert_eq!(out, 42);
    }
}

/// 目的：与 `tests_tokio` 相同，但运行在 compio 后端上。
///
/// 实施策略：创建 compio 运行时（`Runtime::new()` 默认开启全部 driver），
/// 在其 `block_on` 上下文内构造运行时**值**，再调用业务函数。
///
/// 通过依据：`double_via_runtime(&rt, 21) == 42`；`sum_stack_data(&rt) == 10`；
/// `generic_two_tasks(&rt, 21) == 63`；`local_rc_double(&rt, 21) == 42`。
#[cfg(all(test, feature = "demo-compio"))]
mod tests_compio {
    use super::*;

    /// 目的：验证最基础的「spawn + block_on」业务函数在 compio 值上可用。
    ///
    /// 实施策略：创建 compio 运行时，在其 `block_on` 上下文内构造 [`CapRt`]
    /// 值，再调用 [`double_via_runtime`]。
    ///
    /// 通过依据：返回 21 * 2 == 42 且无 panic。
    #[test]
    fn business_fn_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = CapRt::current();
            double_via_runtime(&value, 21)
        });
        assert_eq!(out, 42);
    }

    /// 目的：验证放松 `'static` 后的 `TrBlockOn` 在 compio 值上同样可用。
    ///
    /// 实施策略：在 compio 运行时上下文内构造 [`BlockOnRt`] 值，调用
    /// [`sum_stack_data`]。
    ///
    /// 通过依据：返回 1 + 2 + 3 + 4 == 10；若 `'static` 约束未放松，
    /// 本测试将无法编译。
    #[test]
    fn borrow_block_on_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = BlockOnRt::current();
            sum_stack_data(&value)
        });
        assert_eq!(out, 10);
    }

    /// 目的：与 tokio 侧同名测试相同，验证单一约束覆盖多个 future 的收益在
    /// compio 后端同样成立。
    ///
    /// 实施策略：在 compio 运行时上下文内调用 [`generic_two_tasks`]，传入
    /// 只声明 `BLOCK_ON | SPAWN_SEND` 的 [`CapRt`] 值。
    ///
    /// 通过依据：返回 `21 + 21 * 2 == 63` 且无 panic；编译通过本身即证明
    /// 一条 `Rt: TrSpawnSend` 约束可覆盖函数内部两种不同的 future。
    #[test]
    fn generic_bound_covers_multiple_futures() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = CapRt::current();
            generic_two_tasks(&value, 21)
        });
        assert_eq!(out, 63);
    }

    /// 目的：验证 compio 后端上本地投递同样已并入运行时值。
    ///
    /// 实施策略：在 compio 运行时上下文内构造 [`LocalRt`] 值，调用
    /// [`local_rc_double`]（`Rc` 任务 + `run_until`）。
    ///
    /// 通过依据：取回 21 * 2 == 42；若队列没有被这个值驱动，测试会挂起。
    #[test]
    fn local_delivery_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = LocalRt::current();
            local_rc_double(&value, 21).await
        });
        assert_eq!(out, 42);
    }
}

/// 编译期能力检查（Tag 严格模式）的负向演示集合。
///
/// 每一条 `compile_fail` 文档测试对应一句「**不能做到什么**」：能力声明
/// 没有覆盖到的操作，一律在**编译期**被拒绝，而不是运行期悄悄出错。
/// 正向演示（可以做到什么）见 `examples/{tokio,compio}_demo/` 下各 cap 的
/// smoke test。
///
/// v0.4 值化之后，负向集合覆盖**两类门控**：
///
/// 1. **能力位门控**：`Runtime<CAPS>` 是否实现某项能力 trait 的条件化 impl
///    （`spawn` / `block_on` / `spawn_local` / `delay`）；
/// 2. **值语义门控**：能力方法只能在**构造出来的值**上调用，而没有上下文时
///    「构造」这一步是运行期失败，不是编译期失败（见
///    [`no_context_construction`]——它刻意**不是** `compile_fail`）。
///
/// 这些负向演示与后端无关：能力位掩码与 trait 约束定义在基础 crate
/// `abs_art`，两个后端实例行为一致。
pub mod strict_mode_check {
    /// (a) 只声明 `BLOCK_ON`，在值上调用 `spawn`（需要 `SPAWN_SEND` 位）→
    /// 编译错误。
    ///
    /// **验证什么**：能力位 `SPAWN_SEND` 真的门控了 `TrSpawnSend` 的条件化
    /// impl——注意调用点形状是 **`rt.spawn(..)`（值上方法调用）**，与业务代码
    /// 的真实写法一致。
    ///
    /// **为什么必须失败**：`Runtime::<{ BLOCK_ON }>::current()` 的 `CAPS = 1`，
    /// 不满足标记 `[(); 1]: HasSpawnSend`，因此该类型**没有** `TrSpawnSend`
    /// 实现；即便 `TrSpawnSend` 已在作用域内，方法解析也找不到可用实现
    /// （E0599：方法存在但 trait bound 不满足）。
    ///
    /// ```compile_fail,E0599
    /// use abs_art_demo::{BLOCK_ON, Runtime, TrSpawnSend};
    ///
    /// // 只声明了 BLOCK_ON：这个值不满足 TrSpawnSend 的 impl 条件
    /// let rt = Runtime::<{ BLOCK_ON }>::current();
    /// let _ = rt.spawn(async { 1 });
    /// ```
    pub mod no_spawn_without_send_cap {}

    /// (b) 零能力 `Runtime<0>` 上调用 `block_on` → 编译错误。
    ///
    /// **验证什么**：零能力是「最小权限」的极端形态——`Runtime<0>` 是合法的
    /// 运行时值，但不实现任何能力 trait。
    ///
    /// **为什么必须失败**：`CAPS = 0` 不含 `BLOCK_ON` 位，
    /// `[(); 0]: HasBlockOn` 不成立，`TrBlockOn` 的实现不存在
    /// （E0599：方法存在但 trait bound 不满足）。
    ///
    /// 零能力值仍可自省后端身份（`TrAsyncRuntime::about`，见
    /// `examples/{tokio,compio}_demo/cap_zero.rs` 的正向演示）。
    ///
    /// ```compile_fail,E0599
    /// use abs_art_demo::{Runtime, TrBlockOn};
    ///
    /// let rt = Runtime::<0>::current();
    /// let _ = rt.block_on(async { 1 });
    /// ```
    pub mod zero_caps_no_block_on {}

    /// (c) `spawn` 要求 future 是 `Send`：投递**捕获**了 `Rc` 的任务 →
    /// 编译错误。
    ///
    /// **验证什么**：`TrSpawnSend::spawn` 的方法级约束 `F: Send`。
    ///
    /// **为什么必须失败**：`Rc` 是 `!Send`，`async move` 把它**移动**进 future
    /// 后，future 本身也 `!Send`，不满足方法级约束——编译器报
    /// `future cannot be sent between threads safely`，并在 `required by a
    /// bound in abs_art_demo::TrSpawnSend::spawn` 处指出正是这个约束。
    ///
    /// 注意：`Rc` 必须被真正移进 future；若只写 `let _ = rc;`（通配符模式
    /// 不移动），future 仍是 `Send`，可以编译。
    ///
    /// ```compile_fail
    /// use std::rc::Rc;
    ///
    /// use abs_art_demo::{CapRt, TrSpawnSend};
    ///
    /// // CapRt 确实实现了 TrSpawnSend；失败只因为 future 不是 Send
    /// let rt = CapRt::current();
    /// let rc = Rc::new(1); // !Send：被 async move 捕获后，future 不是 Send
    /// let _ = rt.spawn(async move {
    ///     let _x = rc;
    ///     1
    /// });
    /// ```
    pub mod spawn_requires_send {}

    /// (d) `spawn` 要求 future 是 `'static`：借用局部数据 → 编译错误。
    ///
    /// **验证什么**：`TrSpawnSend::spawn` 的方法级约束 `F: 'static`。
    ///
    /// **为什么必须失败**：`async` 块捕获了对局部 `data` 的**借用**，future 的
    /// 生存期被钉在当前栈帧上；而 `spawn` 要求任务脱离当前栈帧运行，借用必然
    /// 不成立（E0373：async block may outlive the current function）。
    ///
    /// 对照：[`crate::sum_stack_data`] 用 `block_on` 可以借用——`TrBlockOn`
    /// 的 `'static` 约束已放松（上一提交），但 `TrSpawnSend` **没有**放松。
    ///
    /// ```compile_fail,E0373
    /// use abs_art_demo::{CapRt, TrSpawnSend};
    ///
    /// let rt = CapRt::current();
    /// let data = vec![1, 2, 3];
    /// let _ = rt.spawn(async {
    ///     data.iter().sum::<i32>() // data 是借用，非 'static
    /// });
    /// ```
    pub mod spawn_requires_static {}

    /// (e) 没写 `SPAWN_LOCAL` 时 `spawn_local` → 编译错误（v0.4 新增的负向
    /// 用例）。
    ///
    /// **验证什么**：本地投递的**调用点**真的被 `SPAWN_LOCAL` 位门控。
    /// 这是 v0.3 做不到的负向用例——当时调用点在独立的 `LocalScope` 值上，
    /// 能力位只影响 `Runtime::local_scope()` 这个入口，而 `LocalScope::new()`
    /// 是公开的，拿不到位也能绕过去投递。v0.4 把本地队列并回运行时值，
    /// [`TrLocalScope`](crate::TrLocalScope) 的条件化 impl 直接由本位门控，
    /// 于是「没写下来 → 真的调不了」第一次在**调用点**成立。
    ///
    /// **为什么必须失败**：`Runtime::<{ BLOCK_ON }>::current()` 的 `CAPS = 1`
    /// 不含 `SPAWN_LOCAL`（位 3），`[(); 1]: HasSpawnLocal` 不成立，因此这个值
    /// 不实现 `TrLocalScope`（E0599：方法存在但 trait bound 不满足）。
    ///
    /// ```compile_fail,E0599
    /// use abs_art_demo::{BLOCK_ON, Runtime, TrLocalScope};
    ///
    /// // 没写 SPAWN_LOCAL → 这个值不满足 TrLocalScope 的 impl 条件
    /// let rt = Runtime::<{ BLOCK_ON }>::current();
    /// let _ = rt.spawn_local(async { 1 });
    /// ```
    pub mod spawn_local_requires_declaration {}

    /// (f) 没有运行时上下文时构造运行时值（`current()`）——**不是**编译错误。
    ///
    /// 这一条刻意**不**写成 `compile_fail`：`current()` 的类型完全正确，编译器
    /// 没有任何理由拒绝它；拒绝它的是**运行期事实**——调用点不在后端运行时
    /// 上下文内。
    ///
    /// - tokio 后端：`current()` 走 `Handle::current()`，无上下文时 panic
    ///   （信息含 `no reactor running`）；
    /// - compio 后端：同样要求已进入的运行时上下文（经由 `with_current`）。
    ///
    /// 这固定了 v0.4 的使用契约：**运行时值必须由持有环境的一方构造**，它不可能
    /// 像 v0.3 的类型标签那样在任意位置凭空写出来。代价是集成方多了一步「构造并
    /// 传递值」，收益是「哪个运行时、哪条本地队列」由手上的值回答，而不是靠约定。
    ///
    /// 「必须处于上下文内」这一条无法用类型系统表达，因此只作说明，不作为编译期
    /// 负向用例。
    ///
    /// ```no_run
    /// use abs_art_demo::current;
    ///
    /// // 编译通过；运行期 panic：此处不在任何后端运行时上下文内。
    /// let _value = current();
    /// ```
    pub mod no_context_construction {}
}
