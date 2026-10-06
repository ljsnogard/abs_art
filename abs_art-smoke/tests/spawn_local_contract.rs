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
//! # 骨架：造运行时值 → `rt.local_scope()` → 驱动作用域
//!
//! 本地队列是**线程独占**的资源，本轮抽象层把它放回一个独立的值
//! [`TrLocalScope`]（各后端的 `LocalScope`），取得路径只有一条：
//! `Runtime<CAPS>::local_scope()`（要求 `CAPS` 含 `SPAWN_LOCAL`）。于是三个后端的
//! 骨架形状统一，差别只在最外层的驱动写法：
//!
//! ```text
//! OUTER_DRIVER(rt.block_on(scope.run_until(user_future)))
//!   ├─ tokio  : rt.block_on(..)      —— 作用域持有的 LocalSet 由 scope.run_until 驱动
//!   ├─ compio : rt.block_on(..)      —— scope.run_until 即 future 本身（队列归运行时）
//!   └─ smol   : smol::block_on(..)   —— 作用域持有的 LocalExecutor 由 scope.run_until 驱动
//! ```
//!
//! # 调用形状的三代对照（判定标准一字未改）
//!
//! | 步骤 | 原设计 | 中间版（全并进运行时值） | 本轮 |
//! | --- | --- | --- | --- |
//! | 造值 | `LocalScope::new()` | `abs_art_<backend>::current()` | `abs_art_<backend>::current()` |
//! | 取作用域 | 不存在（值即作用域） | 不存在独立作用域 | `rt.local_scope()` |
//! | 本地投递 | `scope.spawn_local(..)` | `rt.spawn_local(..)` | `scope.spawn_local(..)` |
//! | 异步驱动 | `scope.run_until(..)` | `rt.run_until(..)` | `scope.run_until(..)` |
//! | 阻塞驱动（D） | `scope.block_on(..)`（旧 `TrLocalScope::block_on`） | `rt.block_on(..)`（`TrBlockOn`） | `scope.block_on(..)`（`TrLocalScope::block_on`） |
//!
//! # D 用例：本轮调的是 `TrLocalScope::block_on`，不是 `TrBlockOn::block_on`
//!
//! 两个「`block_on`」**分工不同**，本轮把这一点钉死：
//!
//! - [`TrLocalScope::block_on`]（宿主是**作用域**）：阻塞当前线程，并且**驱动本地
//!   队列**直到传入的 future 完成。D 用例走的是这一条。
//! - [`TrBlockOn::block_on`]（宿主是**运行时值**）：只阻塞等待，**不驱动任何本地
//!   队列**。它只该用来等「已经有人在推进的东西」。
//!
//! 因此 D 用例的判定标准仍然是「把探针 A 的结果 `42` 交回上层」，但它的证据变了：
//! 能交回 `42` 恰恰说明投递到本作用域的 `!Send` 任务在 `scope.block_on` 期间被驱动了
//! ——这正是 `TrBlockOn::block_on` **做不到**的事。
//!
//! # `TrSpawnSend` 在矩阵里的位置：compio **不**实现它
//!
//! compio 没有「跨线程全局工作队列」这种能力（它的运行时本身就是线程本地的），因此
//! 抽象层删掉了 compio 的 `impl TrSpawnSend`，并把它写进 `abs_art-compio` 的文档。
//! 本文件的四条契约**都只依赖 `TrLocalScope`**（本地投递 + 本地驱动），所以 compio
//! 的四格照样保留、照样要求通过；任何依赖 `TrSpawnSend` 的用例都**不该**出现在
//! compio 这一列——本文件没有这样的用例，也不打算为了「凑齐矩阵」而伪造一个。

// 两个 trait 都是**值方法**的调用前提：`scope.spawn_local` / `scope.run_until` /
// `scope.block_on` 来自 `TrLocalScope`。值本身由各后端的 `current()` 交出，作用域由
// `rt.local_scope()` 交出，具体类型名在这里都不需要出现。
//
// `TrSpawnSend` 只在最后那条「按后端分组」的类型级用例里出现。
use abs_art::{TrLocalScope, TrSpawnSend};
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
/// `TrLocalScope::block_on`（D 用例）内部走 `block_in_place`，而 `block_in_place`
/// 在 current_thread 运行时上会 panic，因此 D 用例必须用多线程运行时（与
/// `abs_art-tokio` 的实现前提一致）。
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

/// 目的：验证 tokio 后端上 [`TrLocalScope::block_on`]——**作用域**的阻塞入口在等待
/// 期间会驱动它自己的本地队列（注意：这不是 `TrBlockOn::block_on`，后者只等待、
/// 不驱动队列）。
///
/// 实施策略：用多线程运行时（`block_in_place` 的前提）的外层 `rt.block_on` 提供
/// 运行时上下文，在其中经 `rt.local_scope()` 取作用域，再调
/// `scope.block_on(probe_a_handle_driven(&scope))`。
///
/// 通过依据：交回 `42`。若 `TrLocalScope::block_on` 只等待而不驱动本地队列，投递
/// 出去的 `!Send` 任务永远推不动，await 句柄会挂起，由 `run_case` 的超时判失败。
#[test]
fn tokio_d_block_on_entry() {
    assert_a(
        "tokio",
        run_case("tokio｜D 作用域 block_on", || {
            let rt = tokio_mt_rt()?;
            rt.block_on(async {
                let value = abs_art_tokio::current();
                let scope = value.local_scope();
                scope.block_on(probe_a_handle_driven(&scope))
            })
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

/// 目的：验证 compio 后端上 [`TrLocalScope::block_on`]——**作用域**的阻塞入口同样
/// 能驱动它钉住的那条队列（compio 侧它落到 `self.rt_.block_on`，自己 `enter` 并 tick）。
///
/// 实施策略：在 compio 运行时上下文内用 `abs_art_compio::current()` 造值，经
/// `rt.local_scope()` 取作用域，调 `scope.block_on(probe_a_handle_driven(&scope))`。
///
/// 通过依据：交回 `42`。注意本用例走的是 `TrLocalScope::block_on`，不是
/// `TrBlockOn::block_on`。
#[test]
fn compio_d_block_on_entry() {
    assert_a(
        "compio",
        run_case("compio｜D 作用域 block_on", || {
            let rt = compio_rt()?;
            rt.block_on(async {
                let value = abs_art_compio::current();
                let scope = value.local_scope();
                scope.block_on(probe_a_handle_driven(&scope))
            })
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

/// 目的：验证 smol 后端上 [`TrLocalScope::block_on`]——**作用域**的阻塞入口驱动
/// 它自己那条本地队列（smol 无任何先决条件）。
///
/// 实施策略：经 `rt.local_scope()` 取作用域后直接调
/// `scope.block_on(probe_a_handle_driven(&scope))`（内部为
/// `smol::block_on(local.run(fut))`）。
///
/// 通过依据：交回 `42`。注意本用例走的是 `TrLocalScope::block_on`，不是
/// `TrBlockOn::block_on`。
#[test]
fn smol_d_block_on_entry() {
    assert_a(
        "smol",
        run_case("smol｜D 作用域 block_on", || {
            let value = abs_art_smol::current();
            let scope = value.local_scope();
            scope.block_on(probe_a_handle_driven(&scope))
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
