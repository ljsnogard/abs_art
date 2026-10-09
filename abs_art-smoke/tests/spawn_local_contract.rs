//! 跨后端本地投递（`spawn_local`）行为契约的集成冒烟测试。
//!
//! 本文件里**测试体一份都没有**——测试体全部在 `abs_art_smoke::probe`，且泛型于
//! `S: TrLocalScope` 并接收**作用域值** `&S`。本文件只提供三样东西：
//!
//! 1. 每个后端的「创建运行时 + 造运行时值 + 取得作用域 + 驱动」骨架（这是三个后端
//!    **唯一**无法统一的地方：创建运行时的代码本来就属于集成方）；
//! 2. 三组共享的断言函数，保证三个后端用的是**同一套判定标准**；
//! 3. 每个用例一条独立测试，使得矩阵里哪个格子红了可以直接看出来。
//!
//! # 骨架：造运行时值 → `rt.local_scope()` → 驱动**本线程**队列
//!
//! 本地队列是**线程独占**的资源，抽象层用一个独立的值 [`TrLocalScope`]（各后端的
//! `LocalScope`）表示它，取得路径只有一条：`Runtime<CAPS>::local_scope()`（要求
//! `CAPS` 含 `SPAWN_LOCAL`）。三个后端交出的是**本线程那条队列的别名**：`Clone` 只是
//! 多一个别名、同一线程上多次调用拿到同一条。于是三个后端的骨架形状统一，差别只在
//! 最外层的驱动写法：
//!
//! ```text
//! OUTER_DRIVER(scope.run_until(user_future))
//!   ├─ tokio  : rt.block_on(..)      —— 驱动本线程的 LocalSet（返回的 future 需在上下文里 poll）
//!   ├─ compio : rt.block_on(..)      —— scope.run_until 即 future 本身（队列归运行时）
//!   └─ smol   : smol::block_on(..)   —— 驱动本线程的 LocalExecutor
//! ```
//!
//! # 调用形状的三代对照（判定标准一字未改）
//!
//! | 步骤 | 原设计 | 中间版（全并进运行时值） | 本轮 |
//! | --- | --- | --- | --- |
//! | 造值 | `LocalScope::new()` | `abs_art_<backend>::current()` | `abs_art_<backend>::current()` |
//! | 取作用域 | 不存在（值即作用域） | 不存在独立作用域 | `rt.local_scope()`（线程本地队列的别名） |
//! | 本地投递 | `scope.spawn_local(..)` | `rt.spawn_local(..)` | `scope.spawn_local(..)` |
//! | 异步驱动 | `scope.run_until(..)` | `rt.run_until(..)` | `scope.run_until(..)` |
//! | 阻塞驱动（D） | `scope.block_on(..)`（旧 `TrLocalScope::block_on`） | `rt.block_on(..)`（`TrBlockOn`） | `rt.block_on(scope.run_until(..))`：**阻塞在运行时值、驱动在作用域** |
//!
//! # D 用例：阻塞与驱动是两件事，必须显式组合
//!
//! 早先那条「作用域的阻塞入口」`TrLocalScope::block_on` 已被删除：tokio 的
//! `block_in_place` 在 `LocalSet` 内被 tokio 自己禁止（源码注释：「in a LocalSet,
//! where it is _not_ okay to block」），于是「`scope.block_on(f)`」在三个后端上分别是
//! panic / 只驱动自己那条队列 / 顺带驱动整个运行时——同一个名字三种承诺。抽象层保留了
//! 两个**不同**的东西：
//!
//! - [`TrLocalScope::run_until`]（宿主是**作用域**）：**驱动本线程队列**直到传入的
//!   future 完成；
//! - [`TrBlockOn::block_on`]（宿主是**运行时值**）：只阻塞等待，**不驱动任何本地队列**。
//!
//! D 用例把两者**组合**起来用：`value.block_on(scope.run_until(probe))`。它的判定标准
//! 不变（把探针 A 的 `42` 交回上层），证据却更精确：只 `value.block_on(probe)` 会挂起，
//! 正因为 `block_on` 自己不驱动队列。
//!
//! > 后来抽象层又补回了「阻塞 + 驱动队列」，但**换了机制**：
//! > [`TrLocalScope::block_on_local`] 不使用任何运行时阻塞原语（tokio / smol 是
//! > 「驱动队列 + 纯 park」，compio 是自己 tick），因此可以在本地队列的驱动栈内调用。
//! > 它的契约测试在三个后端各自的 `local_scope` 单测里，不在本文件（本文件的 D 继续
//! > 守住「组合写法」这条既有证据）。
//!
//! # `TrSpawnSend` 在矩阵里的位置：compio **不**实现它
//!
//! compio 没有「跨线程全局工作队列」这种能力（它的运行时本身就是线程本地的），因此
//! 抽象层删掉了 compio 的 `impl TrSpawnSend`，并把它写进 `abs_art-compio` 的文档。
//! 本文件的四条契约**都只依赖 `TrLocalScope`**（本地投递 + 本地驱动），所以 compio
//! 的四格照样保留、照样要求通过；任何依赖 `TrSpawnSend` 的用例都**不该**出现在
//! compio 这一列——本文件没有这样的用例，也不打算为了「凑齐矩阵」而伪造一个。

// 三个 trait 都是**值方法**的调用前提：`scope.spawn_local` / `scope.run_until` 来自
// `TrLocalScope`，`value.block_on` 来自 `TrBlockOn`。作用域由 `rt.local_scope()` 交出；
// D 用例外，其余用例连具体类型名都不需要出现。
//
// `TrSpawnSend` 只在最后那条「按后端分组」的类型级用例里出现。
use abs_art::{TrBlockOn, TrLocalScope, TrSpawnSend};
use abs_art_smoke::{
    LOOP_COUNT, LoopResult, expected_sum, probe_a_handle_driven, probe_b_runtime_driven,
    probe_c_detach_survives, run_case,
};

// ── 运行时的创建（集成方职责，三个后端各不相同）────────────────────────────

/// 创建 tokio 单线程运行时。
///
/// 走 `rt.block_on(scope.run_until(..))` 的用例不需要 `block_in_place`，单线程足够，
/// 而且更贴近「本地队列必须由同一条线程驱动」这条事实。
fn tokio_rt() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

/// 创建 tokio 多线程运行时。
///
/// 保留给「必须在运行时上下文内构造值」的场景；D 用例现在走上下文外的
/// `Runtime::with_handle`，不再需要它。
#[allow(dead_code)]
fn tokio_mt_rt() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

/// 创建 compio 运行时。
fn compio_rt() -> Result<compio::runtime::Runtime, String> {
    compio::runtime::Runtime::new().map_err(|e| e.to_string())
}

// ── 共享断言（三个后端必须用同一套判定标准）────────────────────────────────

/// 统一的断言：探针 A 必须交回 `42`。
fn assert_a(backend: &str, outcome: Result<u32, String>) {
    match outcome {
        Ok(value) => assert_eq!(value, 42, "{backend}：句柄交回 {value}，预期 42"),
        Err(e) => panic!("{e}"),
    }
}

/// 统一的断言：探针 B 的结果必须逐条等于「编号 + 该编号的预期求和」。
fn assert_b(backend: &str, outcome: Result<Vec<LoopResult>, String>) {
    match outcome {
        Ok(results) => {
            let expected: Vec<LoopResult> = (0..LOOP_COUNT)
                .map(|idx| (idx, expected_sum(idx)))
                .collect();
            assert_eq!(results, expected, "{backend}：循环结果与预期不符");
        }
        Err(e) => panic!("{e}"),
    }
}

/// 统一的断言：探针 C 的回报值必须等于第 0 个循环的预期求和。
fn assert_c(backend: &str, outcome: Result<u32, String>) {
    match outcome {
        Ok(value) => assert_eq!(
            value,
            expected_sum(0),
            "{backend}：detach 后的循环回报值与预期不符"
        ),
        Err(e) => panic!("{e}"),
    }
}

// ── tokio ─────────────────────────────────────────────────────────────────

/// 目的：验证 tokio 后端上「句柄驱动」用例——经**作用域值**投递的 `!Send` 任务，
/// 在宿主 await 其 `JoinHandle` 时完成并交回结果。
///
/// 实施策略：在独立线程内创建单线程运行时；在运行时上下文内用
/// `abs_art_tokio::current()` 造运行时值，经 `rt.local_scope()` 取得作用域，再用
/// `rt.block_on(scope.run_until(..))` 驱动 `probe_a_handle_driven(&scope)`。
///
/// 通过依据：句柄交回 `42` 且任务内改写的本地状态一致；超时或 `Err` 判失败。
#[test]
fn tokio_a_handle_driven() {
    assert_a(
        "tokio",
        run_case("tokio｜A 句柄驱动", || {
            let rt = tokio_rt()?;
            rt.block_on(async {
                let value = abs_art_tokio::current();
                let scope = value.local_scope();
                scope.run_until(probe_a_handle_driven(&scope)).await
            })
        }),
    );
}

/// 目的：验证 tokio 后端上「作用域驱动」用例——若干消费消息死循环任务，在宿主
/// **没有 poll 任何 `JoinHandle`** 时仍能持续消费消息、收到最后一条退出消息后跳出
/// 循环，并把结果经 `JoinHandle` 交回上层。
///
/// 实施策略：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_b_runtime_driven`，
/// 该探测体先等循环自己发出的完成回执，之后才逐个 await 句柄。
///
/// 通过依据：3 个句柄都返回 `Ok` 且逐条等于预期；若本地队列只能靠 poll 句柄推进，
/// 宿主会在等待回执处被永久阻塞，由 `run_case` 的超时判失败。
#[test]
fn tokio_b_runtime_driven() {
    assert_b(
        "tokio",
        run_case("tokio｜B 作用域驱动", || {
            let rt = tokio_rt()?;
            rt.block_on(async {
                let value = abs_art_tokio::current();
                let scope = value.local_scope();
                scope.run_until(probe_b_runtime_driven(&scope)).await
            })
        }),
    );
}

/// 目的：验证 tokio 后端上「`detach` 后存活」用例——`spawn_local` 之后立即
/// `detach()` 的循环任务，在句柄被消费后仍继续被调度直到自行收尾。
///
/// 实施策略：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_c_detach_survives`；
/// 句柄已消费，循环只能经通道回报自己的求和。
///
/// 通过依据：回报值等于预期；若 `detach` 实际取消了任务，通道会关闭并判失败。
#[test]
fn tokio_c_detach_survives() {
    assert_c(
        "tokio",
        run_case("tokio｜C detach 后存活", || {
            let rt = tokio_rt()?;
            rt.block_on(async {
                let value = abs_art_tokio::current();
                let scope = value.local_scope();
                scope.run_until(probe_c_detach_survives(&scope)).await
            })
        }),
    );
}

/// 目的：验证 tokio 后端上「运行时值阻塞 + 作用域驱动队列」这套**可移植组合**
/// ——阻塞由 [`TrBlockOn::block_on`] 提供，队列由 [`TrLocalScope::run_until`] 提供。
///
/// 实施策略：在运行时上下文**之外**用 `Runtime::with_handle` 造运行时值，经
/// `local_scope()` 取作用域，再调 `value.block_on(scope.run_until(probe))`。
///
/// 通过依据：交回 `42`。若组合里漏掉 `run_until`（只 `value.block_on(probe)`），投递
/// 出去的 `!Send` 任务永远不会被推进，await 句柄会挂起，由 `run_case` 的超时判失败。
///
/// 说明：早先那条作用域阻塞入口 `TrLocalScope::block_on` 已删除（tokio 的
/// `block_in_place` 在 `LocalSet` 内被 tokio 自己禁止，三后端对「`scope.block_on`」
/// 给不出同一个承诺），本用例因此改为测「组合写法」。后来补回的
/// `TrLocalScope::block_on_local` 走的是**另一套机制**，由三个后端的 `local_scope`
/// 单测覆盖，不在本文件里。
#[test]
fn tokio_d_blocking_combo() {
    assert_a(
        "tokio",
        run_case("tokio｜D 阻塞组合", || {
            let rt = tokio_rt()?;
            let value =
                abs_art_tokio::Runtime::<{ abs_art::FULL }>::with_handle(rt.handle().clone());
            let scope = value.local_scope();
            value.block_on(scope.run_until(probe_a_handle_driven(&scope)))
        }),
    );
}

// ── compio ────────────────────────────────────────────────────────────────

/// 目的：验证 compio 后端上「句柄驱动」用例（本地队列归运行时所有，作用域钉住
/// 那份运行时）。
///
/// 实施策略：创建 compio 运行时，在其上下文内用 `abs_art_compio::current()` 造运行时
/// 值，经 `rt.local_scope()` 取作用域，用 `rt.block_on(scope.run_until(..))` 驱动
/// `probe_a_handle_driven`（compio 的 `run_until` 即 future 本身）。
///
/// 通过依据：句柄交回 `42`。
#[test]
fn compio_a_handle_driven() {
    assert_a(
        "compio",
        run_case("compio｜A 句柄驱动", || {
            let rt = compio_rt()?;
            rt.block_on(async {
                let value = abs_art_compio::current();
                let scope = value.local_scope();
                scope.run_until(probe_a_handle_driven(&scope)).await
            })
        }),
    );
}

/// 目的：验证 compio 后端上「作用域驱动」用例——宿主不 poll 句柄时循环任务仍持续
/// 消费消息，并在最后一条退出消息后跳出、经句柄交回结果。
///
/// 实施策略：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_b_runtime_driven`；
/// compio 的队列由运行时自己在 `block_on` 期间 tick，因此「驱动」来自外层上下文。
///
/// 通过依据：3 个句柄结果逐条等于预期；超时判失败。
#[test]
fn compio_b_runtime_driven() {
    assert_b(
        "compio",
        run_case("compio｜B 作用域驱动", || {
            let rt = compio_rt()?;
            rt.block_on(async {
                let value = abs_art_compio::current();
                let scope = value.local_scope();
                scope.run_until(probe_b_runtime_driven(&scope)).await
            })
        }),
    );
}

/// 目的：验证 compio 后端上「`detach` 后存活」用例。
///
/// 实施策略：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_c_detach_survives`；
/// 作用域钉住的那份运行时在自己的 `block_on` 里持续推进队列。
///
/// 通过依据：循环经通道回报的求和等于预期；若 detach 取消了任务则判失败。
#[test]
fn compio_c_detach_survives() {
    assert_c(
        "compio",
        run_case("compio｜C detach 后存活", || {
            let rt = compio_rt()?;
            rt.block_on(async {
                let value = abs_art_compio::current();
                let scope = value.local_scope();
                scope.run_until(probe_c_detach_survives(&scope)).await
            })
        }),
    );
}

/// 目的：验证 compio 后端上「运行时值阻塞 + 作用域驱动队列」这条组合同样成立
/// （compio 侧 `run_until` 即 future 本身，队列由运行时的 `block_on` 循环 tick）。
///
/// 实施策略：在运行时上下文之外用 `Runtime::with_runtime` 造运行时值，经
/// `local_scope()` 取作用域，调 `value.block_on(scope.run_until(probe))`。
///
/// 通过依据：交回 `42`。compio 的 `block_on` 会 `enter` 出上下文并在循环里 tick
/// 执行器，因此这条组合在 compio 上没有 tokio 那样的上下文/多线程前提。
#[test]
fn compio_d_blocking_combo() {
    assert_a(
        "compio",
        run_case("compio｜D 阻塞组合", || {
            let rt = compio_rt()?;
            let value =
                abs_art_compio::Runtime::<{ abs_art_compio::FULL }>::with_runtime(rt.clone());
            let scope = value.local_scope();
            value.block_on(scope.run_until(probe_a_handle_driven(&scope)))
        }),
    );
}

// ── smol ──────────────────────────────────────────────────────────────────

/// 目的：验证 smol 后端上「句柄驱动」用例——执行器不挂在 `JoinHandle` 上，而是归
/// 作用域值所有（作用域由运行时值交出）。
///
/// 实施策略：用 `abs_art_smol::current()` 造运行时值，经 `rt.local_scope()` 取作用域，
/// 用 `smol::block_on(scope.run_until(..))` 驱动 `probe_a_handle_driven`。
///
/// 通过依据：句柄交回 `42`。
#[test]
fn smol_a_handle_driven() {
    assert_a(
        "smol",
        run_case("smol｜A 句柄驱动", || {
            let value = abs_art_smol::current();
            let scope = value.local_scope();
            smol::block_on(scope.run_until(probe_a_handle_driven(&scope)))
        }),
    );
}

/// 目的：验证 smol 后端上「作用域驱动」用例——**值化之前此格为红**
/// （宿主永久阻塞）。
///
/// 实施策略：用 `smol::block_on(scope.run_until(..))` 驱动 `probe_b_runtime_driven`；
/// `LocalExecutor::run` 在等待宿主 future 期间持续驱动本地队列，因此循环即使不被
/// poll 也能推进。
///
/// 通过依据：3 个句柄结果逐条等于预期；若队列仍只能靠 poll 句柄推进，宿主会被永久
/// 阻塞并由 `run_case` 的超时判失败。
#[test]
fn smol_b_runtime_driven() {
    assert_b(
        "smol",
        run_case("smol｜B 作用域驱动", || {
            let value = abs_art_smol::current();
            let scope = value.local_scope();
            smol::block_on(scope.run_until(probe_b_runtime_driven(&scope)))
        }),
    );
}

/// 目的：验证 smol 后端上「`detach` 后存活」用例——**值化之前此格为红**
/// （任务当场被取消）。执行器归作用域所有后，`detach` 不再连带销毁本地队列。
///
/// 实施策略：用 `smol::block_on(scope.run_until(..))` 驱动 `probe_c_detach_survives`。
///
/// 通过依据：循环经通道回报的求和等于预期；若 detach 仍取消任务，通道会关闭并判失败。
#[test]
fn smol_c_detach_survives() {
    assert_c(
        "smol",
        run_case("smol｜C detach 后存活", || {
            let value = abs_art_smol::current();
            let scope = value.local_scope();
            smol::block_on(scope.run_until(probe_c_detach_survives(&scope)))
        }),
    );
}

/// 目的：验证 smol 后端上「运行时值阻塞 + 作用域驱动队列」这条组合同样成立
/// （`scope.run_until` 负责驱动本线程队列，`smol::block_on` 负责阻塞）。
///
/// 实施策略：经 `rt.local_scope()` 取作用域后调
/// `value.block_on(scope.run_until(probe))`（后者的 `block_on` 即 `smol::block_on`）。
///
/// 通过依据：交回 `42`。若漏掉 `run_until`，投递出去的 `!Send` 任务不会被推进。
#[test]
fn smol_d_blocking_combo() {
    assert_a(
        "smol",
        run_case("smol｜D 阻塞组合", || {
            let value = abs_art_smol::current();
            let scope = value.local_scope();
            value.block_on(scope.run_until(probe_a_handle_driven(&scope)))
        }),
    );
}

// ── 按后端分组：哪些后端有 `TrSpawnSend` ───────────────────────────────────

/// 目的：把「`TrSpawnSend` 只在 tokio / smol 上成立」这条**分组事实**钉进矩阵
/// ——compio 不实现它，也不为它造用例。
///
/// 实施策略：只在编译期断言 tokio / smol 的运行时**值**
/// （`Runtime<{ FULL }>`）实现了 [`TrSpawnSend`]。compio 一侧刻意**不写**任何断言：
/// 它没有跨线程全局工作队列，抽象层因此删掉了那个 `impl`——「compio 不实现」这件事
/// 无法用正实现表达，`abs_art-compio` 的 crate 文档用 `compile_fail` 文档测试来钉它。
/// 本用例是那条分工在**契约矩阵**里的对应物：它保证矩阵里任何将来依赖
/// `TrSpawnSend` 的用例都只能被放进 tokio / smol 两列。
///
/// 通过依据：编译通过即为通过。若哪天 tokio / smol 的运行时值不再实现
/// `TrSpawnSend`（或 `HasSpawnSend` 门控写错），本用例无法编译。
#[test]
fn spawn_send_is_present_on_tokio_and_smol_only() {
    fn assert_spawn_send<Runtime_>()
    where
        Runtime_: TrSpawnSend,
    {
    }

    assert_spawn_send::<abs_art_tokio::Runtime<{ abs_art::FULL }>>();
    assert_spawn_send::<abs_art_smol::Runtime<{ abs_art::FULL }>>();
    // compio：本行不存在，且在可预见的将来也不该存在（见本用例文档）。
}
