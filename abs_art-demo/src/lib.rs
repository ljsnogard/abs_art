//! `abs_art-demo`：演示「业务库零泛型穿透 + 集成方通过 Cargo.toml 选后端」。
//!
//! - `src/lib.rs`（业务库）：只依赖 [`abs_art_bridge`]（当前后端实例，见下），
//!   业务函数接收「**运行时值** + **本地作用域**」两件套——计时与时刻从
//!   `&R` 上取（`rt.delay` / `rt.now`），本地投递从 `&S` 上取
//!   （`scope.spawn_local` / `scope.run_until` / `scope.block_on`）；
//! - `src/main.rs`（二进制）：负责**创建运行时 → 构造运行时值 → 取得作用域 →
//!   传给业务库**——它是唯一允许感知后端的地方（创建哪个运行时的代码必须与
//!   所选后端一致）。
//!
//! # 后端选择（本 crate 的 features）
//!
//! `Cargo.toml` 里 `demo-tokio` 与 `demo-compio` **互斥**，一次构建只能启用
//! 一个（缺省是 **compio 组**，与 `abs_art-bridge` 的缺省后端一致）：
//!
//! ```text
//! cargo run -p abs_art-demo                                              # compio 组（默认）
//! cargo run -p abs_art-demo --no-default-features --features demo-tokio  # tokio 组
//! ```
//!
//! 本 crate 把同一个 `abs_art-bridge` 以两个 backend 实例化（重命名依赖
//! `bridge_tokio` / `bridge_compio`），`src/lib.rs` 用 `pub use ... as
//! abs_art_bridge` 把「当前后端实例」统一暴露为 `abs_art_bridge`——业务代码
//! 的写法与后端无关。
//!
//! # 两件套：运行时**值**与本地**作用域**
//!
//! 最新的抽象把两个生命周期完全不同的东西**分开**：
//!
//! | 关切 | 挂在哪 | 怎么用 |
//! | --- | --- | --- |
//! | 投递到全局（跨线程）工作队列 | 运行时**值** | `rt.spawn(..)` |
//! | 阻塞等待 / 周期源 / 时刻 | 运行时**值** | `rt.block_on(..)` / `rt.delay(..)` / `rt.now()` |
//! | 投递 `!Send` 任务 / 驱动本地队列 | **作用域** `LocalScope` | `scope.spawn_local(..)` / `scope.run_until(..)` / `scope.block_on(..)` |
//!
//! 取得作用域的**唯一入口**是 `Runtime<CAPS>::local_scope()`，而它要求
//! `[(); CAPS]: HasSpawnLocal`——于是「开始用本地投递」这件事必然写在类型别名
//! 里、出现在 diff 与 code review 中。作用域**不**实现 [`TrDelay`] /
//! [`TrClock`] / [`TrTime`]：计时与时刻一律从运行时值上取。
//!
//! 能力位共**六个**（`0..=63`）：`BLOCK_ON` / `DELAY` / `SPAWN_SEND` /
//! `SPAWN_LOCAL` / `SPAWN_BLOCKING` / **`CLOCK`**（`1 << 5`）。其中 `CLOCK`
//! 只负责「读时刻」：`impl TrClock` 要求 `[(); CAPS]: HasClock`，而
//! `TrTime: TrDelay + TrClock`，所以 `interval` / `timeout` **同时要 `DELAY`
//! 与 `CLOCK`**。**没写 `CLOCK` 就没有 `now()`**——只写 `DELAY` 只能睡，不能读表。
//! 本 crate 里凡是调用 `now()` / `timeout(..)` 的类型别名都写
//! `{ DELAY | CLOCK }`（见 [`DelayRt`]）。
//!
//! 另有一条分工必须记住：**`Runtime::block_on(f)` 不驱动本地队列**，
//! `scope.block_on(f)` 才驱动队列。两者语义不同，见各函数文档。
//!
//! # 共同子集：为什么 `TrSpawnSend` 是**按后端可用**的
//!
//! compio **没有**跨线程全局工作队列：它的 `spawn` 投的是本线程运行时的队列，
//! 因此 compio 的运行时值**不实现** [`TrSpawnSend`]。于是「三后端共用的业务
//! 代码」只能建立在**共同子集**上：
//!
//! ```text
//! TrBlockOn + TrDelay + TrClock / TrTime + TrSpawnBlocking + TrLocalScope
//! （不含 TrSpawnSend；掩码上 TrTime 还要 CLOCK 那一位，见 [`DelayRt`]）
//! ```
//!
//! # 「全能力」必须按后端具名：裸 `FULL` 与 `TokioFull` / `CompioFull`
//!
//! `abs_art` 的 `FULL` 是**位集合**意义上的「全部」（六位全置，`63`），不代表
//! 每个后端都兑现得了。`abs_art-bridge` 因此给两套名字：
//!
//! - **裸名 `FULL`** = **默认后端**的完整能力集。在 workspace 的 feature 并集
//!   构建下，默认后端是 compio，于是裸 `FULL == 59`（不含 `SPAWN_SEND`）——
//!   拿它当 tokio 组的「全能力」会**静默少一位**，`rt.spawn(..)` 直接不可用；
//! - **具名 `TokioFull` / `CompioFull` / `SmolFull`** = 各后端**自己的**完整能力
//!   集，只要该后端的 feature 开启就存在，因此并集构建下依然精确。本 crate 的
//!   [`FullRt`] 与 [`current()`] 按 `demo-*` 分组取用它们。
//!
//! compio 侧还有一条更强的纪律：**声明了 `SPAWN_SEND` 位就是静态失败**。
//! `abs_art-compio` 用 `CompioCaps_` 断言（`#[diagnostic::on_unimplemented]`）
//! 把 `Runtime<CAPS>` 的**类型定义**卡在「不含 `SPAWN_SEND`」上，于是
//! `Runtime::<{ CompioFull | SPAWN_SEND }>`（即 `63`）在**构造点**就报 E0277，
//! 而不是拖到调用 `spawn` 时才说「没有这个方法」。原始错误原文见
//! [`strict_mode_check`] 的 `compio_rejects_spawn_send_at_construction`。
//!
//! # 本 crate 据此把业务项分成三组
//!
//! - **共同子集**（两种后端都能编译）：[`BlockOnRt`] / [`DelayRt`] /
//!   [`LocalRt`] / [`BlockingRt`] / [`FullRt`] 与它们对应的业务函数；
//! - **tokio 组**（`#[cfg(feature = "demo-tokio")]`）：依赖 `TrSpawnSend` 的
//!   `CapRt` / `double_via_runtime` / `generic_two_tasks`（这三个只在 tokio 组存在，
//!   故此处用代码体而非文档链接）；
//! - **compio 组**（`#[cfg(feature = "demo-compio")]`）：没有 `TrSpawnSend`
//!   时的替代路径 `local_three_tasks`——改用本地作用域投递。
//!
//! # Examples（每种 cap 一个 smoke test，按后端分组）
//!
//! `examples/tokio_demo/` 与 `examples/compio_demo/` 各 7 个 demo，一一对应，
//! 文档注释里写明「要验证什么 / 可以做到什么 / 不能做到什么」：
//!
//! - `cap_block_on.rs`：`BLOCK_ON` —— 最小能力 + `TrBlockOn` 放松 `'static`
//!   后可以驱动借用栈数据的 future / 返回借用引用；
//! - `cap_spawn_send.rs`：`BLOCK_ON | SPAWN_SEND` —— tokio 侧是真正的跨线程
//!   投递；**compio 侧是反向演示**：它没有跨线程全局队列，声明 `SPAWN_SEND`
//!   位即静态失败（E0277），示范改用本地作用域投递；
//! - `cap_spawn_local.rs`：本地投递 —— `rt.local_scope()` 取得作用域，
//!   `scope.spawn_local(..)` 投递 `!Send` 任务，`scope.run_until(..)` /
//!   `scope.block_on(..)` 驱动队列；
//! - `cap_delay.rs`：`DELAY | CLOCK` —— 时间驱动与 time driver 前提；
//!   两后端都演示 `now()`，因此都必须写 `CLOCK`；
//! - `cap_spawn_blocking.rs`：`BLOCK_ON | SPAWN_BLOCKING` —— 阻塞线程池与
//!   异步侧共存；
//! - `cap_full.rs`：**本后端的**完整能力集（`TokioFull` = `63` /
//!   `CompioFull` = `59`）——六个能力位（含 `SPAWN_LOCAL` 声明位）+ 两件套
//!   协同 + 后端自省（`rt.tag()` / `rt.about()`）；
//! - `cap_zero.rs`：`0` —— 零能力边界：`Runtime<0>` 只是值类型，任何能力调用
//!   都是编译错误。
//!
//! 「不能做到什么」的编译期负向演示集中在 [`strict_mode_check`]
//! （`compile_fail` 文档测试，`cargo test --doc` 验证，按后端分别生效）。
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
compile_error!("abs_art-demo：必须启用 demo-tokio 或 demo-compio 之一（默认 demo-compio）");

/// 能力项与类型再导出：业务库对外暴露与后端无关的入口。
///
/// 这样 doctest / 下游代码可以统一写 `abs_art_demo::BLOCK_ON`、
/// `abs_art_demo::LocalScope` 而不必关心当前启用的是哪个后端实例。
///
/// 注意 [`TrLocalScope`] 在这里是**作用域**的契约：它的宿主是
/// `Runtime::local_scope()` 交出的 `LocalScope` 值，不是运行时值本身。
///
/// [`FULL`] 是**裸名**（= 默认后端的完整能力集），保留它是为了说明差异：
/// 全能力别名请用 [`FullCaps`] / [`FullRt`]，它们按 `demo-*` 分组取用
/// `TokioFull` / `CompioFull`。
pub use abs_art_bridge::{
    BLOCK_ON, CLOCK, DELAY, FULL, RuntimeTag, SPAWN_BLOCKING, SPAWN_LOCAL, SPAWN_SEND,
    TrAsyncRuntime, TrBlockOn, TrClock, TrDelay, TrJoinHandle, TrLocalScope,
    TrSpawnBlocking, TrSpawnSend, TrTime,
};

// ── 后端相关的名字：用**具名别名**绑定到本 crate 的 demo-* 分组 ──────────
//
// 刻意**不**重导出 bridge 的裸名 `Runtime` / `LocalScope` / `current`：bridge 的裸名
// 跟着它自己的**缺省后端**走，而 `cargo test --workspace` 会把 bridge 的 feature
// 取并集（bridge 的缺省后端 + 本 crate 的 `bridge_*` 依赖所用的后端）。用具名别名与
// `demo-*` 分组绑定之后，「当前后端」只由本 crate 的 feature 决定，不受并集影响。

/// 当前分组（tokio）的运行时与作用域类型（含具名别名）。
#[cfg(feature = "demo-tokio")]
pub use abs_art_bridge::{
    TokioLocalScope, TokioLocalScope as LocalScope, TokioRuntime,
    TokioRuntime as Runtime,
};

/// 当前分组（compio）的运行时与作用域类型（含具名别名）。
#[cfg(feature = "demo-compio")]
pub use abs_art_bridge::{
    CompioLocalScope, CompioLocalScope as LocalScope, CompioRuntime,
    CompioRuntime as Runtime,
};

/// 当前分组的**完整能力集**（按 `demo-*` 分组取具名常量）。
///
/// - `demo-tokio` → [`TokioFull`](abs_art_bridge::TokioFull) = `63`（含 `SPAWN_SEND`）；
/// - `demo-compio` → [`CompioFull`](abs_art_bridge::CompioFull) = `59`（**不含**
///   `SPAWN_SEND`：compio 没有跨线程全局工作队列，声明该位即静态失败）。
///
/// 刻意**不**用裸名 [`FULL`]：那是**默认后端**的完整能力集。workspace 的 feature
/// 并集下默认后端是 compio，裸 `FULL == 59`，拿它写 tokio 组的「全能力」会
/// **静默少一位**（`rt.spawn(..)` 不可用）。具名常量随各自后端的 feature 存在，
/// 因此在并集构建下也精确。
#[cfg(feature = "demo-tokio")]
pub use abs_art_bridge::TokioFull as FullCaps;

/// 当前分组的**完整能力集**（compio 版，见 tokio 分支的说明）。
#[cfg(feature = "demo-compio")]
pub use abs_art_bridge::CompioFull as FullCaps;

/// 用**当前分组**的运行时上下文构造全能力运行时值。
///
/// 等价于 `Runtime::<{ FullCaps }>::current()`（即 tokio 组的
/// `Runtime::<{ TokioFull }>` / compio 组的 `Runtime::<{ CompioFull }>`）；单独
/// 给出是为了让示例与 doctest 不必写类型参数（`Runtime::current()` 写在表达式
/// 位置会 `E0284`）。
///
/// # Panics
///
/// 不在当前分组的运行时上下文内时 panic（与后端 `current()` 的前提一致）。
pub fn current() -> Runtime<{ FullCaps }> {
    Runtime::<{ FullCaps }>::current()
}

// =====================================================================
// 共同子集：不含 TrSpawnSend，两种后端都能编译。
// =====================================================================

/// 最小能力声明：只需要 `block_on` 一种能力。
///
/// 对应 `examples/{tokio,compio}_demo/cap_block_on.rs`；也展示 `TrBlockOn`
/// 放松 `'static` 约束带来的实际收益——两个后端都支持（tokio 的
/// `Handle::block_on` 与 compio 的 `Runtime::block_on` 均接受非 `'static`
/// 的 future）。
pub type BlockOnRt = Runtime<{ BLOCK_ON }>;

/// 计时与时刻的能力声明：`DELAY | CLOCK`（**两位**）。
///
/// `CLOCK`（`1 << 5`，见 [`CLOCK`]）是「读时刻」的独立能力位：各后端的
/// `impl TrClock` 要求 `[(); CAPS]: HasClock`，而 `TrTime: TrDelay + TrClock`，
/// 因此 `now()` 与 `timeout(..)` / `interval(..)` **都要这一位**。
///
/// 只写 `DELAY` 时 `delay` 仍然可用，但 `rt.now()` 会被编译期拒绝（trait bound
/// 不满足）——本别名从 `{ DELAY }` 改成 `{ DELAY | CLOCK }` 正是为此。
/// `interval` 同样由这两个位门控，只是本 crate 未从 bridge 再导出 `TrInterval`，
/// 故示例只用 `delay` / `now` / `timeout` 三者。
pub type DelayRt = Runtime<{ DELAY | CLOCK }>;

/// 本地投递的能力声明：`BLOCK_ON | DELAY | SPAWN_LOCAL`。
///
/// - `SPAWN_LOCAL` 是**声明位**：它让 `Runtime<CAPS>::local_scope()` 可用
///   （该关联函数要求 `[(); CAPS]: HasSpawnLocal`）；
/// - `DELAY` 用于演示「计时从 `&R` 取、投递从 `&S` 取」这两条互不重叠的路径；
/// - `BLOCK_ON` 让集成方还能在同一个值上做阻塞等待。
///
/// 注意这个值**不含** `SPAWN_SEND`：本地投递与跨线程投递是两件事，本别名只
/// 承诺前者。
pub type LocalRt = Runtime<{ BLOCK_ON | DELAY | SPAWN_LOCAL }>;

/// 阻塞池的能力声明：`BLOCK_ON | SPAWN_BLOCKING`。
pub type BlockingRt = Runtime<{ BLOCK_ON | SPAWN_BLOCKING }>;

/// 全能力声明（**tokio 组**）：`Runtime<{ TokioFull }>`（`63`）。
///
/// 这里必须写**具名**的 `TokioFull`（经 [`FullCaps`] 解析），不能用裸名
/// [`FULL`]：裸 `FULL` 是**默认后端**的完整能力集，在 workspace 的 feature 并集
/// 构建下等于 compio 的 `59`，会让 tokio 组静默少一位 `SPAWN_SEND`
/// （`rt.spawn(..)` 直接不可用）。`TokioFull` 只要 tokio 后端的 feature 开启就
/// 存在，因此在并集构建下也精确。
#[cfg(feature = "demo-tokio")]
pub type FullRt = Runtime<{ abs_art_bridge::TokioFull }>;

/// 全能力声明（**compio 组**）：`Runtime<{ CompioFull }>`（`59`）。
///
/// `CompioFull` **不含** `SPAWN_SEND`：compio 没有跨线程全局队列，它兑现不了那
/// 一位。这也是本组的 `FullRt` 能通过编译的原因——若把它写成
/// `Runtime::<{ CompioFull | SPAWN_SEND }>`（数值上等于 `abs_art::FULL`，`63`），
/// 失败发生在**构造点**：`CompioCaps_` 静态断言给出 E0277
/// （见 [`strict_mode_check::compio_rejects_spawn_send_at_construction`]）。
///
/// 因此「全能力值可以调 `spawn`」只在 tokio 组成立，且这一点由**具名常量**而不是
/// `FULL` 这个名字表达。
#[cfg(feature = "demo-compio")]
pub type FullRt = Runtime<{ abs_art_bridge::CompioFull }>;

/// 共同子集示例业务函数：在这个运行时值上 `block_on` 一个**借用栈上数据**的
/// future（非 `'static`）。
///
/// 旧约束（`F: Future + 'static` 且 `F::Output: 'static`）下，`async` 块借用
/// 局部 `data` 无法通过编译；[`TrBlockOn`] 的约束放松为 `F: Future` 后，
/// 这类「只在本栈帧内同步等一个 async 结果」的代码得以成立。
///
/// 本函数只用到运行时**值**（不涉及作用域）：「阻塞等待」与「驱动本地队列」
/// 的分工见 [`local_double_blocking`]。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{BlockOnRt, sum_stack_data};
///
/// let rt = BlockOnRt::current();
/// assert_eq!(sum_stack_data(&rt), 10);
/// ```
pub fn sum_stack_data(rt: &BlockOnRt) -> usize {
    let data = [1usize, 2, 3, 4];
    rt.block_on(async { data.iter().sum() })
}

/// 共同子集示例业务函数：**两件套**的正面展示——计时从 `&R` 取，投递从 `&S` 取。
///
/// 具体分工：
///
/// 1. `rt.delay(..)`：睡眠来自**运行时值**（计时不属于本地调度）；
/// 2. `scope.run_until(..)`：驱动**作用域**持有的本地队列；
/// 3. `scope.spawn_local(..)`：把 `!Send` 的 `Rc` 任务投到那条队列。
///
/// 本函数是 `async fn`：`run_until` 返回的 future 需要放在「已处于该后端运行时
/// 上下文」的位置 await（由集成方的最外层 `rt.block_on(..)` 或
/// `scope.block_on(..)` 提供）。
///
/// `R` 只要求 [`TrDelay`]、`S` 只要求 [`TrLocalScope`]：这是共同子集的一部分
/// （**不含** `TrSpawnSend`），因此本函数在 tokio 与 compio 下逐字相同。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{LocalRt, timed_local_double};
///
/// # async fn demo() {
/// let value = LocalRt::current();
/// let scope = value.local_scope();
/// assert_eq!(timed_local_double(&value, &scope, 21).await, 42);
/// # }
/// ```
pub async fn timed_local_double<R, S>(rt: &R, scope: &S, x: i32) -> i32
where
    R: TrDelay,
    S: TrLocalScope,
{
    // 计时从运行时**值**上取——作用域上没有 delay（那是另一件关切）。
    rt.delay(core::time::Duration::from_millis(1)).await;

    scope
        .run_until(async {
            // Rc 是 !Send：只有「线程本地」的队列能承载这样的任务。
            let rc = std::rc::Rc::new(x);
            // 投递点也在作用域上：队列归它所有。
            scope.spawn_local(async move { *rc * 2 }).await.unwrap()
        })
        .await
}

/// 共同子集示例业务函数：同步地、**由作用域驱动队列**地取回本地任务结果。
///
/// 这里刻意使用 [`TrLocalScope::block_on`]（**恢复了**的能力），而不是
/// `Runtime::block_on`：
///
/// - `scope.block_on(f)` 在等待期间**持续驱动本地队列**，`f` 里 spawn 的本地
///   任务因此能完成；
/// - `rt.block_on(f)` 只等待，不驱动任何队列——若把本函数体换成
///   `rt.block_on(..)`，内部 `spawn_local` 的任务可能永远不被推进。
///
/// # Panics
///
/// 各后端的先决条件与其 `TrBlockOn` 实现一致（tokio 需要多线程运行时且调用点
/// 已处于运行时上下文内），不满足时由后端 panic。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{LocalRt, local_double_blocking};
///
/// let value = LocalRt::current();
/// let scope = value.local_scope();
/// assert_eq!(local_double_blocking(&scope, 21), 42);
/// ```
pub fn local_double_blocking<S>(scope: &S, x: i32) -> i32
where
    S: TrLocalScope,
{
    scope.block_on(async {
        let rc = std::rc::Rc::new(x);
        scope.spawn_local(async move { *rc * 2 }).await.unwrap()
    })
}

/// 共同子集示例业务函数：一次性睡眠与超时都从**运行时值**取。
///
/// 抽象层把计时分成三层（`TrDelay` 睡眠 / `TrClock` 时刻 / `TrTime` 周期与超时），
/// 本函数把其中可观测的两条跑一遍：
///
/// 1. `rt.delay(d)`：至少等待 `d`（睡眠是「至少」而不是「精确」）；
/// 2. `rt.timeout(大期限, 立即就绪的 future)`：内层先完成 → `Ok`；
/// 3. `rt.timeout(1ms, rt.delay(50ms))`：期限先到 → `Err`（内层 future 被丢弃）。
///
/// 返回值：(延迟是否足额, 内层赢的超时是否成功, 期限先到是否被判超时)。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{DelayRt, delay_and_timeout};
///
/// # async fn demo() {
/// let rt = DelayRt::current();
/// let (delayed, fast_ok, slow_elapsed) = delay_and_timeout(&rt).await;
/// assert!(delayed && fast_ok && slow_elapsed);
/// # }
/// ```
pub async fn delay_and_timeout<R>(rt: &R) -> (bool, bool, bool)
where
    R: TrTime,
{
    let start = std::time::Instant::now();
    rt.delay(core::time::Duration::from_millis(5)).await;
    let delayed = start.elapsed() >= core::time::Duration::from_millis(5);

    // 内层立即完成：超时不应触发。
    let fast_ok = rt
        .timeout(core::time::Duration::from_secs(5), async { 7u8 })
        .await
        .is_ok();

    // 内层要睡 50ms、期限只有 1ms：期限先到，返回 Err。
    let slow_elapsed = rt
        .timeout(
            core::time::Duration::from_millis(1),
            rt.delay(core::time::Duration::from_millis(50)),
        )
        .await
        .is_err();

    (delayed, fast_ok, slow_elapsed)
}

/// 共同子集示例业务函数：向这个运行时值的阻塞线程池投两个重活。
///
/// `spawn_blocking` 是共同子集的一部分——三个后端都提供阻塞池，且都从**运行时
/// 值**上取用。返回两个任务结果之和。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{BlockingRt, blocking_pair};
///
/// # async fn demo() {
/// let rt = BlockingRt::current();
/// assert_eq!(blocking_pair(&rt).await, 570);
/// # }
/// ```
pub async fn blocking_pair(rt: &BlockingRt) -> usize {
    /// 模拟 CPU 密集计算：`0..n` 的平方和。
    fn heavy_compute_(n: usize) -> usize {
        (0..n).map(|i| i * i).sum()
    }

    let h1 = rt.spawn_blocking(move || heavy_compute_(10));
    let h2 = rt.spawn_blocking(move || heavy_compute_(10));
    h1.await.unwrap() + h2.await.unwrap()
}

// =====================================================================
// tokio 组：依赖 TrSpawnSend 的业务项（compio 不实现该 trait）。
// =====================================================================

/// 业务库声明的能力组合：`block_on` + `spawn_send` 两种能力。
///
/// **只在 tokio 组存在**：`TrSpawnSend` 的语义是「投递到跨线程全局工作队列」，
/// 而 compio 没有这样的队列（它的 `spawn` 投的是本线程运行时的队列），因此
/// compio 的运行时值不实现该 trait——把这个别名定义在 compio 组只会得到一个
/// 永远不能被 `spawn` 的空壳。
#[cfg(feature = "demo-tokio")]
pub type CapRt = Runtime<{ BLOCK_ON | SPAWN_SEND }>;

/// tokio 组示例业务函数：在这个运行时值上 spawn 一个任务计算 `x * 2`，再
/// `block_on` 等待结果。
///
/// 该函数**没有任何泛型参数**——运行时由**值参数** `rt` 提供，能力边界通过
/// [`CapRt`] 的类型参数静态声明，编译期完成校验，运行期零开销。
///
/// 本函数依赖 [`TrSpawnSend`]，因此只对实现了跨线程全局队列的后端（tokio /
/// smol）成立；compio 侧的替代写法见 `local_three_tasks`（compio 组专用）。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{CapRt, double_via_runtime};
///
/// let rt = CapRt::current();
/// assert_eq!(double_via_runtime(&rt, 21), 42);
/// ```
#[cfg(feature = "demo-tokio")]
pub fn double_via_runtime(rt: &CapRt, x: i32) -> i32 {
    rt.block_on(async move {
        let handle = rt.spawn(async move { x * 2 });
        handle.await.unwrap()
    })
}

/// tokio 组泛型业务函数：只写**一个** `Rt: TrSpawnSend + TrBlockOn` 约束，就
/// spawn 两种不同的 future（其中第二个是无法命名的 `async {}` 块），再
/// `block_on` 等待结果。
///
/// 这是把自由参数 `F` 从 trait 泛型列表移到 [`TrSpawnSend::spawn`] 方法级泛型
/// 后获得的能力：更早的版本需要为每个 future 各写一条约束
/// （`Rt: TrSpawnSend<F1> + TrSpawnSend<F2>`），而函数内部 `async` 块的类型
/// 外部**无法命名**，`F2` 那条约束根本写不出来。本函数不感知任何后端，
/// `Rt` 由调用方（集成方）以**值**的形式给出。
///
/// 同样依赖 [`TrSpawnSend`]，因此只对 tokio / smol 成立。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{CapRt, generic_two_tasks};
///
/// let rt = CapRt::current();
/// assert_eq!(generic_two_tasks(&rt, 21), 63);
/// ```
#[cfg(feature = "demo-tokio")]
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

// =====================================================================
// compio 组：没有 TrSpawnSend 时的替代路径。
// =====================================================================

/// compio 组示例业务函数：**没有 `TrSpawnSend` 时该怎么办**——改用本地作用域
/// 投递。
///
/// compio 的运行时是线程本地的（内部全是 `Rc`，`!Send`），没有跨线程全局工作
/// 队列，因此它的运行时值不实现 [`TrSpawnSend`]：`rt.spawn(..)` 这条路在
/// compio 上根本不存在。可行的替代是**本地作用域投递**：
///
/// - [`TrLocalScope::spawn_local`] 不要求 `F: Send`（队列本来就在本线程），
///   因此同一条「投三个任务再聚合」的业务在这里换个入口就能落地；
/// - 局限也写清楚：这三个任务在本线程上**交错**执行，不会像 tokio 那样分散到
///   多个 worker 核——跨线程并行正是 compio 不承诺、也无法用 `TrSpawnSend`
///   承诺的部分。
///
/// 返回值即 `x + x*2 + x*3`，与 tokio 组的 `concurrent_sum` 在数值上一致，
/// 便于对照「同一份业务语义、两条不同的投递路径」。
///
/// # Examples
///
/// ```no_run
/// use abs_art_demo::{FullRt, local_three_tasks};
///
/// # async fn demo() {
/// let value = FullRt::current();
/// let scope = value.local_scope();
/// assert_eq!(local_three_tasks(&scope, 7).await, 42);
/// # }
/// ```
#[cfg(feature = "demo-compio")]
pub async fn local_three_tasks<S>(scope: &S, x: i32) -> i32
where
    S: TrLocalScope,
{
    scope
        .run_until(async {
            let h1 = scope.spawn_local(async move { x });
            let h2 = scope.spawn_local(async move { x * 2 });
            let h3 = scope.spawn_local(async move { x * 3 });
            h1.await.unwrap() + h2.await.unwrap() + h3.await.unwrap()
        })
        .await
}

// =====================================================================
// 测试
// =====================================================================

/// 目的：验证共同子集与 tokio 组业务项在真实 tokio 后端上运行正常
/// （运行时值 + 作用域两件套）。
///
/// 实施策略：由本 crate 选定的演示组（`demo-tokio`）创建 tokio 运行时，在运行时
/// 上下文内**构造运行时值**，再取作用域并把两件套交给业务函数。
///
/// 通过依据：各测试的断言全部成立，且没有 panic / 挂起。
#[cfg(all(test, feature = "demo-tokio"))]
mod tests_tokio {
    use super::*;

    /// 目的：验证「spawn + block_on」这条依赖 `TrSpawnSend` 的业务路径在 tokio
    /// 值上可用（这是 tokio 组特有的能力）。
    ///
    /// 实施策略：创建多线程 tokio 运行时，在其 `block_on` 上下文内构造
    /// [`CapRt`] 值（`current()` 需要环境上下文），再调用 [`double_via_runtime`]。
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

    /// 目的：验证放松 `'static` 后的 `TrBlockOn` 在共同子集函数上可用。
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

    /// 目的：验证单一 `Rt: TrSpawnSend` 约束能覆盖多个 future（含无法命名的
    /// `async` 块）——即把 `F` 移到方法级泛型后真正拿到的收益。
    ///
    /// 实施策略：在 tokio 多线程运行时上下文内调用泛型函数
    /// [`generic_two_tasks`]，传入只声明 `BLOCK_ON | SPAWN_SEND` 的 [`CapRt`]
    /// 值；该函数内部仅凭一条 `Rt: TrSpawnSend` 约束 spawn 两个不同的 future。
    ///
    /// 通过依据：返回 `21 + 21 * 2 == 63`；若 trait 退回「每个 future 一条约束」
    /// 的形状（无法为匿名 future 命名约束），本测试将无法编译。
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

    /// 目的：验证「运行时值 + 作用域」两件套——`rt.local_scope()` 取得作用域、
    /// `scope.spawn_local` 投递 `!Send` 任务、`scope.run_until` 驱动队列，而
    /// `delay` 仍从运行时值上取。
    ///
    /// 实施策略：在 current_thread tokio 运行时上下文内构造 [`LocalRt`] 值，
    /// 取作用域后调用 [`timed_local_double`]（内部 `delay` + `Rc` 本地任务 +
    /// `run_until`）。因为函数内部有 `delay`，运行时须开启 time driver。
    ///
    /// 通过依据：取回 21 * 2 == 42；若本地队列没有被这个作用域驱动，
    /// `run_until` 将永久挂起。
    #[test]
    fn local_delivery_works() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt.block_on(async {
            let value = LocalRt::current();
            let scope = value.local_scope();
            timed_local_double(&value, &scope, 21).await
        });
        assert_eq!(out, 42);
    }

    /// 目的：验证**作用域的** `block_on` 会驱动本地队列（而运行时值的
    /// `block_on` 不会）。
    ///
    /// 实施策略：tokio 的 `TrLocalScope::block_on` 基于 `block_in_place`，因此用
    /// 多线程运行时；在其 `block_on` 上下文内构造 [`LocalRt`] 值、取作用域，
    /// 调用同步的 [`local_double_blocking`]（内部只用 `scope.block_on`）。
    ///
    /// 通过依据：返回 21 * 2 == 42；若驱动点错写成运行时值的 `block_on`，
    /// 内部本地任务不会被推进，测试将挂起。
    #[test]
    fn local_block_on_drives_queue() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async {
            let value = LocalRt::current();
            let scope = value.local_scope();
            local_double_blocking(&scope, 21)
        });
        assert_eq!(out, 42);
    }

    /// 目的：验证计时系列能力（`delay` / `timeout`）都从运行时**值**上取用，
    /// 并按抽象层的语义契约工作。
    ///
    /// 实施策略：在开启 time driver 的 tokio 运行时内构造 [`DelayRt`] 值，调用
    /// [`delay_and_timeout`]：先量 `delay(5ms)` 的墙上耗时，再分别验证「内层
    /// 先完成 → Ok」与「期限先到 → Err」。
    ///
    /// 通过依据：三元组为 `(true, true, true)`；若 time driver 未开启，
    /// `delay` 将永远 pending，测试会挂起。
    #[test]
    fn time_comes_from_runtime_value() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt.block_on(async {
            let value = DelayRt::current();
            delay_and_timeout(&value).await
        });
        assert_eq!(out, (true, true, true));
    }

    /// 目的：验证共同子集里的 `TrSpawnBlocking` 在 tokio 值上可用。
    ///
    /// 实施策略：在 tokio 运行时上下文内构造 [`BlockingRt`] 值，调用
    /// [`blocking_pair`]（内部投两个平方和闭包到阻塞池）。
    ///
    /// 通过依据：返回 `2 * 285 == 570`。
    #[test]
    fn spawn_blocking_works() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async {
            let value = BlockingRt::current();
            blocking_pair(&value).await
        });
        assert_eq!(out, 570);
    }
}

/// 目的：与 `tests_tokio` 对应，验证共同子集业务项在 compio 后端上运行正常，
/// 并钉住「compio 没有 `TrSpawnSend`，替代路径是本地作用域投递」这条精度。
///
/// 实施策略：创建 compio 运行时（`Runtime::new()` 默认开启全部 driver），在其
/// `block_on` 上下文内构造运行时**值**、取得作用域，再调用业务函数。
///
/// 通过依据：各测试的断言全部成立，且没有 panic / 挂起。
#[cfg(all(test, feature = "demo-compio"))]
mod tests_compio {
    use super::*;

    /// 目的：验证「计时 + 本地投递」两件套在 compio 上可用（compio 组的主路径，
    /// 不涉及 `TrSpawnSend`）。
    ///
    /// 实施策略：创建 compio 运行时，在其 `block_on` 上下文内构造 [`LocalRt`]
    /// 值、取作用域，再调用 [`timed_local_double`]。
    ///
    /// 通过依据：返回 21 * 2 == 42；若 `delay` 的注册点与值的运行时不一致，
    /// 或队列未被驱动，测试会挂起。
    #[test]
    fn business_fn_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = LocalRt::current();
            let scope = value.local_scope();
            timed_local_double(&value, &scope, 21).await
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

    /// 目的：钉住 compio 的替代路径——没有 `TrSpawnSend` 时用本地作用域投递，
    /// 得到与 tokio 组 `concurrent_sum` 相同的业务结果。
    ///
    /// 实施策略：在 compio 运行时上下文内取 [`FullRt`] 值的作用域，调用
    /// [`local_three_tasks`]（投三个任务再聚合）。
    ///
    /// 通过依据：返回 `7 + 14 + 21 == 42`；本测试能编译本身即证明这条路径不需要
    /// `TrSpawnSend`。
    #[test]
    fn local_replacement_for_spawn_send_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = FullRt::current();
            let scope = value.local_scope();
            local_three_tasks(&scope, 7).await
        });
        assert_eq!(out, 42);
    }

    /// 目的：验证 compio 下作用域的 `block_on` 同样驱动本地队列。
    ///
    /// 实施策略：在 compio 运行时上下文内取 [`LocalRt`] 值的作用域，调用同步的
    /// [`local_double_blocking`]。
    ///
    /// 通过依据：返回 21 * 2 == 42；compio 的队列由运行时自己驱动，因此这条路径
    /// 没有 tokio 那样的多线程前提。
    #[test]
    fn local_block_on_drives_queue() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = LocalRt::current();
            let scope = value.local_scope();
            local_double_blocking(&scope, 21)
        });
        assert_eq!(out, 42);
    }

    /// 目的：验证计时系列能力在 compio 值上可用（compio 的计时注册是环境式的，
    /// 本测试固定「值的 `block_on` 与注册点同源」这一实践）。
    ///
    /// 实施策略：在 compio 运行时上下文内构造 [`DelayRt`] 值，调用
    /// [`delay_and_timeout`]。
    ///
    /// 通过依据：三元组为 `(true, true, true)`。
    #[test]
    fn time_comes_from_runtime_value() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = DelayRt::current();
            delay_and_timeout(&value).await
        });
        assert_eq!(out, (true, true, true));
    }

    /// 目的：验证共同子集里的 `TrSpawnBlocking` 在 compio 值上可用。
    ///
    /// 实施策略：在 compio 运行时上下文内构造 [`BlockingRt`] 值，调用
    /// [`blocking_pair`]。
    ///
    /// 通过依据：返回 `2 * 285 == 570`。
    #[test]
    fn spawn_blocking_works() {
        let rt = compio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let value = BlockingRt::current();
            blocking_pair(&value).await
        });
        assert_eq!(out, 570);
    }
}

/// 编译期能力检查（Tag 严格模式）的负向演示集合。
///
/// 每一条 `compile_fail` 文档测试对应一句「**不能做到什么**」：能力声明没有
/// 覆盖到的操作，一律在**编译期**被拒绝。正向演示（可以做到什么）见
/// `examples/{tokio,compio}_demo/` 下各 cap 的 smoke test。
///
/// 最新抽象下，负向集合覆盖**三类门控**：
///
/// 1. **能力位门控**：`Runtime<CAPS>` 是否实现某项能力 trait 的条件化 impl
///    （`spawn` / `block_on` / `delay`）；
/// 2. **作用域门控**：`local_scope()` 是取得本地投递能力的**唯一入口**，它要求
///    `CAPS` 含 `SPAWN_LOCAL`——所以「没写下来 → 连作用域都拿不到 → 更没有
///    `spawn_local` 可调」；
/// 3. **后端静态断言**（本轮更新）：compio 的 `Runtime<CAPS>` 在**类型定义**上要求
///    `[(); CAPS]: CompioCaps_`，即 `CAPS` **不得含 `SPAWN_SEND` 位**。声明了就在
///    **构造点**报 E0277（`#[diagnostic::on_unimplemented]` 的人话文案），而不是
///    拖到调用 `spawn` 时才以「没有这个方法」的形式暴露。
///
/// 另外有一条刻意**不是** `compile_fail` 的用例（`no_context_construction`）：
/// 没有上下文时构造运行时值是**运行期**失败，不是编译期失败。
pub mod strict_mode_check {
    /// (a) 只声明 `BLOCK_ON`，在值上调用 `spawn`（需要 `SPAWN_SEND` 位）→
    /// 编译错误。**仅 tokio 组**：它钉住的是 tokio 后端的能力位门控。
    ///
    /// **验证什么**：能力位 `SPAWN_SEND` 真的门控了 `TrSpawnSend` 的条件化
    /// impl——调用点形状是 `rt.spawn(..)`（值上方法调用），与业务代码的真实写法
    /// 一致。
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
    #[cfg(feature = "demo-tokio")]
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
    /// 编译错误。**仅 tokio 组**（它需要 `TrSpawnSend` 存在，否则失败原因会变成
    /// 「方法不存在」，钉不住 `Send` 约束）。
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
    #[cfg(feature = "demo-tokio")]
    pub mod spawn_requires_send {}

    /// (d) `spawn` 要求 future 是 `'static`：借用局部数据 → 编译错误。
    /// **仅 tokio 组**。
    ///
    /// **验证什么**：`TrSpawnSend::spawn` 的方法级约束 `F: 'static`。
    ///
    /// **为什么必须失败**：`async` 块捕获了对局部 `data` 的**借用**，future 的
    /// 生存期被钉在当前栈帧上；而 `spawn` 要求任务脱离当前栈帧运行，借用必然
    /// 不成立（E0373：async block may outlive the current function）。
    ///
    /// 对照：[`crate::sum_stack_data`] 用 `block_on` 可以借用——`TrBlockOn`
    /// 的 `'static` 约束已放松，但 `TrSpawnSend` **没有**放松。
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
    #[cfg(feature = "demo-tokio")]
    pub mod spawn_requires_static {}

    /// (e) 没写 `SPAWN_LOCAL` 时**连作用域都取不到** → 编译错误。
    ///
    /// **验证什么**：取得作用域的唯一入口 `Runtime<CAPS>::local_scope()` 真的被
    /// 能力位门控。这是最新抽象下最干净的一条门控：调用点不在「绕不绕得过去」上
    /// 纠缠——**没有声明，就没有作用域值**。
    ///
    /// **为什么必须失败**：`Runtime::<{ BLOCK_ON }>::current()` 的 `CAPS = 1`
    /// 不含 `SPAWN_LOCAL`（位 3），`[(); 1]: HasSpawnLocal` 不成立，因此该类型
    /// 没有 `local_scope()` 这个固有方法可调（E0599）。
    ///
    /// ```compile_fail,E0599
    /// use abs_art_demo::{BLOCK_ON, Runtime};
    ///
    /// // 没写 SPAWN_LOCAL → 拿不到作用域，也就没有 spawn_local 可调
    /// let rt = Runtime::<{ BLOCK_ON }>::current();
    /// let _scope = rt.local_scope();
    /// ```
    pub mod local_scope_requires_declaration {}

    /// (f) 没写 `SPAWN_LOCAL` 时在运行时值上直接调 `spawn_local` → 编译错误。
    ///
    /// **验证什么**：本地投递**不再挂在运行时值上**——`spawn_local` 是
    /// [`TrLocalScope`](crate::TrLocalScope) 的方法，宿主是作用域值，因此
    /// 「有没有声明位」与「手上拿的是值还是作用域」两层都拦得住。
    ///
    /// **为什么必须失败**：即便把这个方法名写在运行时值上，`Runtime<{ BLOCK_ON }>`
    /// 也没有 `TrLocalScope` 实现——本地投递的 impl 挂在各后端的 `LocalScope`
    /// 上（E0599：方法存在但 trait bound 不满足）。
    ///
    /// ```compile_fail,E0599
    /// use abs_art_demo::{BLOCK_ON, Runtime, TrLocalScope};
    ///
    /// let rt = Runtime::<{ BLOCK_ON }>::current();
    /// let _ = rt.spawn_local(async { 1 });
    /// ```
    pub mod spawn_local_not_on_runtime_value {}

    /// (g) **静态失败**：compio 上声明 `SPAWN_SEND` 位 → **构造点**即编译错误
    /// （E0277），而不是等到调用 `spawn`。
    ///
    /// **验证什么**：把「compio 没有跨线程全局工作队列」这条**后端事实**提前到
    /// 「值被使用」这一刻。`abs_art-compio` 把静态断言写在 `Runtime` 的**类型定义**
    /// 上（`where [(); CAPS]: CompioCaps_`），因此 `CAPS` 含 `SPAWN_SEND` 位时连值
    /// 都构造不出来：`Runtime::<{ CompioFull | SPAWN_SEND }>`（数值 `63`，等于
    /// `abs_art::FULL`）在**构造调用**处就失败。
    ///
    /// **为什么必须失败**：`CompioCaps_` 只对 `0..=63` 中**不含** `SPAWN_SEND` 位的
    /// 32 个掩码实现（compio 的完整能力集 [`crate::FullCaps`] 是 `59`，不含该位）。
    /// 注意：用裸名 `Runtime::<{ FULL }>` 在 compio 组**不会**报错——裸 `FULL` 是
    /// 默认后端的完整能力集，compio 下它就是 `59`；要演示负例必须用「具名全能力集
    /// **显式或上** `SPAWN_SEND`」。
    ///
    /// **原始错误原文**（`cargo check -p abs_art-demo` 实测，取首条 `E0277`）：
    ///
    /// ```text
    /// error[E0277]: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力
    ///    --> abs_art-demo/examples/_probe_compio_static_fail.rs:9:22
    ///     |
    ///   9 |         let _value = Runtime::<{ CompioFull | SPAWN_SEND }>::current();
    ///     |                      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ 请从 CAPS 中去掉 `abs_art::SPAWN_SEND`；compio 的完整能力集是 `abs_art_compio::FULL`
    ///     |
    ///     = help: the trait `abs_art_compio::caps::CompioCaps_` is not implemented for `[(); 63]`
    ///     = note: compio 的执行器是线程本地的：`Runtime::spawn` 投的是本线程运行时的队列。要投递任务请用 `Runtime::local_scope()` 的 `spawn_local`。
    ///     = note: 本断言只对 `0..=63` 中不含 `SPAWN_SEND` 位的 32 个掩码成立；若 CAPS 里还有 `0..=63` 之外的位置位，同样会看到这条信息。
    /// note: required by a bound in `CompioRuntime`
    ///    --> abs_art-compio/src/lib.rs:277:17
    /// ```
    ///
    /// 随后还有一条**次生**错误（`E0599`，指出 `CompioRuntime<63>` 上
    /// `current` 的 trait bounds 不满足）——人话信息来自前一条 `E0277`，这也正是
    /// 断言必须写在类型定义而非仅固有 impl 上的原因。
    ///
    /// **正确的替代**：改用本地作用域投递——
    /// `let scope = rt.local_scope(); scope.spawn_local(..)`（见
    /// `examples/compio_demo/cap_spawn_send.rs` 的反向演示与
    /// [`crate::local_three_tasks`]）。代价要一并记住：本地任务在**本线程**上
    /// 交错执行，没有跨线程并行。
    ///
    /// ```compile_fail,E0277
    /// use abs_art_demo::{FullCaps, Runtime, SPAWN_SEND};
    ///
    /// // 声明了 SPAWN_SEND 位（59 | 4 == 63）：构造点即静态失败，而非调用 spawn 时
    /// let _value = Runtime::<{ FullCaps | SPAWN_SEND }>::current();
    /// ```
    #[cfg(feature = "demo-compio")]
    pub mod compio_rejects_spawn_send_at_construction {}

    /// (h) 没有运行时上下文时构造运行时值（`current()`）——**不是**编译错误。
    ///
    /// 这一条刻意**不**写成 `compile_fail`：`current()` 的类型完全正确，编译器
    /// 没有任何理由拒绝它；拒绝它的是**运行期事实**——调用点不在后端运行时
    /// 上下文内。
    ///
    /// - tokio 后端：`current()` 走 `Handle::current()`，无上下文时 panic
    ///   （信息含 `no reactor running`）；
    /// - compio 后端：同样要求已进入的运行时上下文。
    ///
    /// 这固定了使用契约：**运行时值必须由持有环境的一方构造**，它不可能像类型
    /// 标签那样在任意位置凭空写出来。代价是集成方多了一步「构造并传递值」，
    /// 收益是「哪个运行时、哪条本地队列、哪个时钟」由手上的值回答，而不是靠约定。
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
