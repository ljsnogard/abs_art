//! `spawn_send`：**本后端不提供**「投递到全局（跨线程）工作队列」这项能力。
//!
//! 本模块没有任何 `impl`：它存在的意义是把「为什么不实现」这条因果留在代码里，并用
//! `compile_fail` 文档测试把它钉住，防止日后有人顺手补上一个名不副实的实现。
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
//! 库侧需要「投递」时，正确的抽象是 [`TrLocalScope`](abs_art::TrLocalScope)：
//!
//! ```
//! use abs_art::{FULL, TrLocalScope};
//! use abs_art_compio::Runtime;
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
//! # 两条钉住用法的文档测试
//!
//! 第一条：compio 的运行时值**不是** [`TrSpawnSend`]——哪天有人补上那个 impl，本测试
//! 会立刻由「编译失败（预期）」变成「编译通过（不符合预期）」而报错：
//!
//! ```compile_fail
//! use abs_art::{FULL, TrSpawnSend};
//!
//! fn assert_tr_spawn_send<T: TrSpawnSend>() {}
//! assert_tr_spawn_send::<abs_art_compio::Runtime<{ FULL }>>();
//! ```
//!
//! 第二条：相应地，运行时值与本地作用域上**都没有** `spawn` 这个方法——它们只提供
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
