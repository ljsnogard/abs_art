//! `abs_art-smoke`：跨异步运行时的 `spawn_local` 行为契约冒烟测试。
//!
//! # 这个 crate 解决什么问题
//!
//! `abs_art` 声称「一次编写，任意异步运行时」，但三个后端底层提供的
//! `spawn_local` 能力**并不一样**：
//!
//! - **tokio**：本地队列归调用方的 `LocalSet` 所有，必须由调用方驱动；
//! - **compio**：运行时本身就是线程本地的，队列归运行时所有并由它在 `block_on`
//!   期间驱动；
//! - **smol**：`smol` 2.x **没有**内建 `spawn_local`，必须自建 `LocalExecutor`
//!   并自己驱动。
//!
//! 这些差异在「宿主怎么用句柄」这一点上会产生**可观测的行为差异**。本 crate 把
//! `smux_v1` 真正依赖的那条契约固化成可执行的测试：同一份测试体
//! （[`probe`] 模块，泛型于 `R: TrLocalScope` 并接收运行时**值** `&R`），
//! 三种运行时的驱动骨架，一次运行给出 3 后端 × 4 用例的对比矩阵。
//!
//! # 被测契约
//!
//! 契约的核心是**「消费消息死循环 + 立即脱离句柄」**这一模式（`smux_v1` 的读 /
//! 写两个循环就是它）：
//!
//! 1. 经 `spawn_local` 投递若干个「循环消费消息、收到最后一个退出消息才跳出」
//!    的任务，任务捕获 `Rc`（`!Send`），因此只能走线程本地队列；
//! 2. **任务的推进不能依赖句柄被 poll**——宿主在 await 任何 `JoinHandle`
//!    *之前*，就能等到循环自己发出的完成回执；
//! 3. 循环的返回值必须能经 `JoinHandle` 正确交回上层；
//! 4. `detach()` 消费句柄之后，循环必须**继续被调度**，直到自己收到退出消息。
//!
//! 第 2、4 条正是 `smux_v1` 的 `dev-notes/connection-20261002-0548.md` §17.9
//! 列为「最高优先级实证项」的两条。
//!
//! # 实测结果
//!
//! | 用例 | tokio | compio | smol |
//! | --- | --- | --- | --- |
//! | A 句柄驱动 | ✅ | ✅ | ✅ |
//! | B 运行时驱动（不 poll 句柄） | ✅ | ✅ | ✅ |
//! | C `detach` 后存活 | ✅ | ✅ | ✅ |
//! | D `rt.block_on` 便捷入口 | ✅ | ✅ | ✅ |
//!
//! 这张全绿的表是**改造后**的结果。本 crate 最初建立时（v0.3 的类型级
//! `spawn_local`）smol 的 B、C 两格是红的：那时执行器被塞在 `JoinHandle` 里，
//! 句柄一旦 poll 不到或被 `detach()` 掉，本地任务就再也推不动。
//!
//! 转绿的关键是把本地队列做成**显式的值**——执行器 / `LocalSet` 由本地队列的
//! 持有者持有，句柄只持有任务本身。本 crate 因此同时充当那次改造的**验收标准**
//! 与回归防线。
//!
//! # v0.4「运行时值化」对本 crate 的影响：只有调用形状
//!
//! v0.3 的本地队列持有者是一个**独立的作用域对象**（各后端的 `LocalScope`），
//! 计时能力挂在**类型**上；v0.4 把两者都并进运行时**值**：
//!
//! | | v0.3 | v0.4 |
//! | --- | --- | --- |
//! | 全局投递 | `<Rt as TrSpawnSend>::spawn(f)` | `rt.spawn(f)` |
//! | 本地投递 | `scope.spawn_local(f)` | `rt.spawn_local(f)` |
//! | 异步驱动本地队列 | `scope.run_until(f)` | `rt.run_until(f)` |
//! | 阻塞驱动本地队列 | `scope.block_on(f)`（`TrLocalScope`） | `rt.block_on(f)`（`TrBlockOn`） |
//! | 睡眠 / 周期 / 超时 | `<Rt as TrTime>::delay(d)` … | `rt.delay(d)` … |
//! | 取时刻 | 无（消费方自备时钟） | `rt.now()`（`TrClock`，与计时器同源） |
//! | 探测体泛型 | `S: TrLocalScope` / `T: TrTime` | `R: TrLocalScope` / `R: TrTime`，收 `&R` |
//!
//! 上面四条 `spawn_local` 契约的**语义与判定标准一个字都没改**：改的只是「能力从
//! 哪里来」。[`time_probe`] 那组则新增了一条 v0.3 无法表达的同源契约
//! （[`time_probe::probe_clock_is_monotonic_and_shares_delay_source`]）。
//!
//! 值化的一个直接后果：运行时值在 tokio / smol 上是 `!Send` 的（内部持
//! `Rc<LocalSet>` / `Rc<LocalExecutor>`），因此只能在 [`harness::run_case`] 的
//! 闭包**内部**构造。[`harness`] 模块文档对此有专门说明。
//!
//! # 为什么需要 [`harness::run_case`] 的超时
//!
//! 这类缺口的典型表现形式是**宿主被永久阻塞**，而不是返回错误。若不给每个用例
//! 加独立线程 + 超时，失败会表现为整个测试进程挂死、拿不到任何可读信息。
//!
//! # 与 `no_std` 的关系
//!
//! 本 crate `publish = false`，只服务于测试，因此直接使用 `std`
//! （`std::thread` / `std::rc`），不参与 `abs_art` 的 `no_std` 约束。

pub mod harness;
pub mod probe;
pub mod time_probe;

pub use harness::{CASE_TIMEOUT, run_case};
pub use probe::{
    LOOP_COUNT, LoopResult, MSG_PER_LOOP, Msg, expected_sum, expected_values,
    probe_a_handle_driven, probe_b_runtime_driven, probe_c_detach_survives,
};
