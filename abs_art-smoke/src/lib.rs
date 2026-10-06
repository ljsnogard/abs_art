//! `abs_art-smoke`：跨异步运行时的本地投递与计时行为契约冒烟测试。
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
//! （[`probe`] 模块，泛型于 `S: TrLocalScope` 并接收**作用域值** `&S`），
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
//! `spawn_local` 契约矩阵（4 用例 × 3 后端）：
//!
//! | 用例 | tokio | compio | smol |
//! | --- | --- | --- | --- |
//! | A 句柄驱动 | ✅ | ✅ | ✅ |
//! | B 作用域驱动（不 poll 句柄） | ✅ | ✅ | ✅ |
//! | C `detach` 后存活 | ✅ | ✅ | ✅ |
//! | D 作用域 `block_on` 入口 | ✅ | ✅ | ✅ |
//!
//! [`time_probe`] 契约矩阵（8 用例 × 3 后端）覆盖 `delay` 的时长与零时长、周期源的
//! 首次与相位、`now()` 与 `delay` 的同源、`timeout` 的两条分支，以及「计时能力挂在
//! 运行时值而不是作用域上」。
//!
//! 上面这张 `spawn_local` 表是**改造后**的结果。本 crate 最初建立时（类型级
//! `spawn_local`）smol 的 B、C 两格是红的：那时执行器被塞在 `JoinHandle` 里，
//! 句柄一旦 poll 不到或被 `detach()` 掉，本地任务就再也推不动。
//!
//! 转绿的关键是把本地队列做成**显式的值**——执行器 / `LocalSet` 由那个作用域值持有，
//! 句柄只持有任务本身。本 crate 因此同时充当那次改造的**验收标准**与回归防线。
//!
//! # 本轮的形状：运行时**值** + 线程本地队列的**别名**
//!
//! 抽象层把两件事分开，本 crate 的骨架照此写成「造运行时值 → `rt.local_scope()` →
//! 驱动本线程队列」：
//!
//! | 关切 | 宿主 | 调用形状 |
//! | --- | --- | --- |
//! | 本地投递（`!Send`） | 作用域 `S: TrLocalScope` | `scope.spawn_local(f)` |
//! | 异步驱动本线程队列 | 作用域 | `scope.run_until(f)` |
//! | 阻塞等待（**不**驱动队列） | 运行时值 | `rt.block_on(f)`（`TrBlockOn::block_on`） |
//! | 阻塞 + 驱动队列（组合） | 两者 | `rt.block_on(scope.run_until(f))` |
//! | 睡眠 / 周期 / 超时 | 运行时值 | `rt.delay(d)` / `rt.interval(p)` / `rt.timeout(d, f)` |
//! | 取时刻 | 运行时值 | `rt.now()`（`TrClock`，与计时器同源） |
//!
//! 「阻塞」与「驱动队列」是本轮要钉的重点，而且它们是**两件事、两个 trait**：作用域上
//! 不再有阻塞入口（tokio 的 `block_in_place` 在 `LocalSet` 内被 tokio 自己禁止，三后端
//! 对 `scope.block_on` 给不出同一个承诺）。D 用例走组合写法；[`time_probe`] 的那组探针
//! 一个本地任务也不投，因此只走运行时值的 `block_on`。
//!
//! 本地投递类探针因此泛型于**作用域** `S: TrLocalScope`，计时类探针泛型于**运行时值**
//! `R: TrTime`——这正是本轮的分工。
//!
//! # compio 不实现 `TrSpawnSend`
//!
//! compio 的运行时是线程本地的，**没有**跨线程全局工作队列，因此抽象层删掉了它的
//! `impl TrSpawnSend`。规则是：任何依赖 `TrSpawnSend` 的用例都不在 compio 这一列
//! 出现，而不是假装它有。本 crate 的四条 `spawn_local` 契约只依赖 `TrLocalScope`
//! （本地投递 + 本地驱动），所以在三个后端上**同等成立**，compio 的四格照常保留。
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
