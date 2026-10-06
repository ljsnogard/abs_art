//! `spawn_send`：**本后端不提供**「投递到全局（跨线程）工作队列」这项能力；该位一旦
//! 被声明就**静态失败**——含 `SPAWN_SEND` 位的 `Runtime<CAPS>` 值一被使用即编译错误。
//!
//! 本模块没有任何 `impl`：它存在的意义是把两条因果留在代码里，并用 `compile_fail`
//! 文档测试钉住，防止日后有人顺手补上一个名不副实的实现：
//!
//! 1. **为什么 compio 不实现 [`TrSpawnSend`]**（下面的第一、二节）；
//! 2. **声明了 `SPAWN_SEND` 之后会怎样**——不是拖到调用点说「没有 `spawn` 方法」，
//!    而是在值一出现时就报一条人话错误（第三节）。
//!
//! # 为什么 compio 不实现 [`TrSpawnSend`]
//!
//! [`TrSpawnSend`] 的契约定的是「投递到**全局（跨线程）**工作队列」。compio 上不存在
//! 这样一条队列：
//!
//! - 它的执行器是 `Rc<Executor>`，**就在运行时实例内部**，绑在创建它的线程上
//!   （`compio::runtime::Runtime` 因此是 `!Send` 的）；
//! - `compio::runtime::Runtime::spawn` 的签名是
//!   `pub fn spawn<F: Future + 'static>(&self, future: F) -> JoinHandle<F::Output>`
//!   （`compio-runtime-0.12.6/src/lib.rs:221`）——**不要求** `F: Send`，因为它投的是
//!   本线程这个运行时的队列，future 也仍在**本线程**上被轮询。
//!
//! 如果硬写一个 `impl TrSpawnSend for Runtime<CAPS>`，抽象就会失真：
//!
//! 1. **假的 `Send` 前置条件**：trait 方法要求 `F: Future + Send`，于是调用方以为自己在
//!    做「跨线程投递」并为此付出证明成本，而事实上任务从未离开这条线程；
//! 2. **假的「全局队列」前提**：库侧写 `R: TrSpawnSend` 时是在声明「我依赖一条可跨线程
//!    共享的队列」。在 compio 上这个前提不存在——连承载它的值都是 `!Send` 的，接手方
//!    根本搬不走。
//!
//! # 声明了 `SPAWN_SEND` 会怎样：一使用就报错（静态失败）
//!
//! 只做到「不写 `impl TrSpawnSend`」是不够的：`Runtime::<{ SPAWN_SEND }>::current()`
//! 照样能构造出一个值，问题要拖到**调用 `spawn` 的那一行**才以「没有 `spawn` 方法」
//! 的形式暴露——离病因很远。本 crate 把失败提前到「值被使用」这一刻：
//!
//! - 本 crate 的 [`FULL`](crate::FULL) `= abs_art::FULL & !SPAWN_SEND = 59`，它是
//!   [`Runtime`](crate::Runtime) 的默认 `CAPS`（**不含** `SPAWN_SEND`）；
//! - 断言写在 [`Runtime`](crate::Runtime) 的**类型定义**上
//!   （`where [(); CAPS]: CompioCaps_`），并重复在它的所有固有入口
//!   （`current` / `with_runtime` / `retag` / `tag` / `runtime` / `local_scope`）与所有
//!   能力 trait 的 impl 上。写在类型定义上是**实测**要求：只挂 impl 时 rustc 走
//!   `E0599`，看不到 [`CompioCaps_`](crate::CompioCaps_) 上的
//!   `#[diagnostic::on_unimplemented]` 文案；
//! - [`CompioCaps_`](crate::CompioCaps_) 只对 `0..=63` 中**不含 `SPAWN_SEND` 位**的 32
//!   个掩码实现，于是错误信息是人话：
//!
//! ```text
//! error[E0277]: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力
//!   |
//!   = note: compio 的执行器是线程本地的：`Runtime::spawn` 投的是本线程运行时的队列。
//!           要投递任务请用 `Runtime::local_scope()` 的 `spawn_local`。
//! ```
//!
//! 被覆盖的入口（任一处出现含该位的掩码都会报同一条信息）：
//!
//! | 类别 | 入口 |
//! | --- | --- |
//! | 构造 | `current` / `with_runtime` |
//! | 换标签 | `retag` / `Clone` |
//! | 能力 trait | `TrBlockOn` / `TrDelay` / `TrClock` / `TrTime` / `TrSpawnBlocking` / `TrAsyncRuntime` |
//! | 本地投递 | `local_scope` |
//!
//! 因此泛型代码里写 `R: TrDelay`、实参却给出 `Runtime<{SPAWN_SEND}>` 时，也是在
//! 「实参不满足 bound」处报这条信息，而不是等到调用点。
//!
//! # 替代写法：本地投递
//!
//! 库侧需要「投递」时，正确的抽象是 [`TrLocalScope`](abs_art::TrLocalScope)：
//!
//! ```
//! use abs_art::TrLocalScope;
//! use abs_art_compio::{FULL, Runtime};
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! let out = rt.block_on(async {
//!     // 本地作用域是 compio 上唯一的投递入口
//!     let scope = Runtime::<{ FULL }>::current().local_scope();
//!     scope.spawn_local(async { 42u8 }).await.unwrap()
//! });
//! assert_eq!(out, 42);
//! ```
//!
//! # 钉住机制的三条文档测试
//!
//! 第一条：声明 `abs_art::FULL`（`63`，含 `SPAWN_SEND` 位）**在构造点**就编译失败，
//! 错误信息就是上面那条人话（而不是「没有 `spawn` 方法」）：
//!
//! ```compile_fail
//! use abs_art_compio::Runtime;
//!
//! let rt = compio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     // E0277: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力
//!     let _value = Runtime::<{ abs_art::FULL }>::current();
//! });
//! ```
//!
//! 第二条：本后端的完整能力集（[`FULL`](crate::FULL)）也**不是** [`TrSpawnSend`]——
//! 「去掉那位」换来的是「用得起来」，而不是「补上了那个 impl」。哪天有人补上，本测试
//! 会由「编译失败（预期）」变成「编译通过（不符合预期）」而报错：
//!
//! ```compile_fail
//! use abs_art::TrSpawnSend;
//! use abs_art_compio::{FULL, Runtime};
//!
//! fn assert_tr_spawn_send<T: TrSpawnSend>() {}
//! assert_tr_spawn_send::<Runtime<{ FULL }>>();
//! ```
//!
//! 第三条：相应地，运行时值与本地作用域上**都没有** `spawn` 这个方法——它们只提供
//! [`spawn_local`](abs_art::TrLocalScope::spawn_local)：
//!
//! ```compile_fail
//! let rt = compio::runtime::Runtime::new().unwrap();
//! rt.block_on(async {
//!     let scope = abs_art_compio::current().local_scope();
//!     let _ = scope.spawn(async { 1u8 }); // 编译错误：没有 spawn
//! });
//! ```
//!
//! [`TrSpawnSend`]: abs_art::TrSpawnSend
