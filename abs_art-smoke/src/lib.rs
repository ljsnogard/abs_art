//! `abs_art-smoke`：跨异步运行时的 `spawn_local` 行为契约冒烟测试。
//!
//! # 这个 crate 解决什么问题
//!
//! `abs_art` 声称「一次编写，任意异步运行时」，但三个后端底层提供的
//! `spawn_local` 能力**并不一样**：
//!
//! - **tokio**：`spawn_local` 必须在 `LocalSet` 上下文内调用，否则 panic；
//!   本地队列由调用方的 `LocalSet` 驱动；
//! - **compio**：运行时本身就是线程本地的，`spawn_local` 与 `spawn` 共用同一条
//!   队列，由运行时自己在 `block_on` 期间驱动；
//! - **smol**：`smol` 2.x **没有**内建 `spawn_local`，`abs_art-smol` 目前的做法
//!   是「每次调用新建一个 [`smol::LocalExecutor`]，执行器随 `JoinHandle` 存活，
//!   句柄被 poll 时驱动它」。
//!
//! 这三种实现方式在「宿主怎么用句柄」这一点上产生了**可观测的行为差异**。
//! 本 crate 把 `smux_v1` 真正依赖的那条契约固化成可执行的测试：同一份测试体
//! （[`probe`] 模块，泛型于 `Rt: TrSpawnLocal`），三种运行时的驱动代码，
//! 一次运行给出 3×3 的对比矩阵。
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
//! # 当前实测结果（本 crate 建立时的基线）
//!
//! | 用例 | tokio | compio | smol |
//! | --- | --- | --- | --- |
//! | A 句柄驱动 | ✅ | ✅ | ✅ |
//! | B 运行时驱动（不 poll 句柄） | ✅ | ✅ | ❌ 宿主永久阻塞 |
//! | C `detach` 后存活 | ✅ | ✅ | ❌ 任务被取消 |
//!
//! **smol 的 B、C 两项失败是刻意的、已知的缺口**，不是本 crate 的缺陷：这个
//! 前置冒烟测试的作用就是把缺口变成一条会红的测试，作为后续「让 `spawn_local`
//! 在三个后端上行为一致」这项改造的验收标准。
//!
//! # 为什么需要 [`harness::run_case`] 的超时
//!
//! 上述缺口的表现形式是**宿主被永久阻塞**，而不是返回错误。若不给每个用例
//! 加独立线程 + 超时，失败会表现为整个测试进程挂死、拿不到任何可读信息。
//!
//! # 与 `no_std` 的关系
//!
//! 本 crate `publish = false`，只服务于测试，因此直接使用 `std`
//! （`std::thread` / `std::rc`），不参与 `abs_art` 的 `no_std` 约束。

pub mod harness;
pub mod probe;

pub use harness::{CASE_TIMEOUT, run_case};
pub use probe::{
    LOOP_COUNT, LoopResult, MSG_PER_LOOP, Msg, expected_sum, expected_values,
    probe_a_handle_driven, probe_b_runtime_driven, probe_c_detach_survives,
};
