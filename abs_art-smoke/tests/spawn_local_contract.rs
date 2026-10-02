//! 跨后端 `spawn_local` 行为契约的集成冒烟测试（路线 1：值化的本地作用域）。
//!
//! 本文件里**测试体一份都没有**——测试体全部在 `abs_art_smoke::probe`，且泛型于
//! `S: TrLocalScope`。本文件只提供三样东西：
//!
//! 1. 每个后端的「创建运行时 + 创建作用域 + 驱动」骨架（这是三个后端**唯一**
//!    无法统一的地方：创建运行时的代码本来就属于集成方）；
//! 2. 三组共享的断言函数，保证三个后端用的是**同一套判定标准**；
//! 3. 每个用例一条独立测试，使得矩阵里哪个格子红了可以直接看出来。
//!
//! # 驱动形态的统一
//!
//! 三个后端的驱动**写法完全相同**，只有最外层的 `OUTER_DRIVER` 不同：
//!
//! ```text
//! OUTER_DRIVER(scope.run_until(user_future))
//!   ├─ tokio  : rt.block_on(..)      —— LocalSet 由 run_until 驱动
//!   ├─ compio : rt.block_on(..)      —— run_until 即 future 本身
//!   └─ smol   : smol::block_on(..)   —— LocalExecutor 由 run_until 驱动
//! ```
//!
//! 另外每个后端还有一条 `scope.block_on(..)` 便捷入口的用例（D）。

use abs_art::TrLocalScope;
use abs_art_compio::LocalScope as CompioScope;
use abs_art_smoke::{
    LOOP_COUNT, LoopResult, expected_sum, probe_a_handle_driven, probe_b_runtime_driven,
    probe_c_detach_survives, run_case,
};
use abs_art_smol::LocalScope as SmolScope;
use abs_art_tokio::LocalScope as TokioScope;

// ── 运行时的创建（集成方职责，三个后端各不相同）────────────────────────────

/// 创建 tokio 单线程运行时。
///
/// 走 `scope.run_until` 的用例不需要 `block_in_place`，单线程足够。
fn tokio_rt() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

/// 创建 tokio 多线程运行时。
///
/// `scope.block_on` 内部走 `block_in_place`，而 `block_in_place` 在单线程运行时上
/// 会 panic，因此 D 用例必须用多线程运行时（与 abs_art-tokio 的 `TrBlockOn` 前提一致）。
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

/// 目的：验证 tokio 后端上「句柄驱动」用例——经 `LocalScope` 投递的 `!Send`
/// 任务，在宿主 await 其 `JoinHandle` 时完成并交回结果。
///
/// 手段：在独立线程内创建单线程运行时与 `LocalScope`，用
/// `rt.block_on(scope.run_until(..))` 驱动 `probe_a_handle_driven`。
///
/// 判定：句柄交回 `42` 且任务内改写的本地状态一致；超时或 `Err` 判失败。
#[test]
fn tokio_scope_a_handle_driven() {
    assert_a(
        "tokio",
        run_case("tokio｜A 句柄驱动", || {
            let rt = tokio_rt()?;
            let scope = TokioScope::new();
            rt.block_on(scope.run_until(probe_a_handle_driven(&scope)))
        }),
    );
}

/// 目的：验证 tokio 后端上「运行时驱动」用例——若干消费消息死循环任务，在宿主
/// **没有 poll 任何 `JoinHandle`** 时仍能持续消费消息、收到最后一条退出消息后跳出
/// 循环，并把结果经 `JoinHandle` 交回上层。
///
/// 手段：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_b_runtime_driven`，该探测
/// 体先等循环自己发出的完成回执，之后才逐个 await 句柄。
///
/// 判定：3 个句柄都返回 `Ok` 且逐条等于预期；若本地队列只能靠 poll 句柄推进，
/// 宿主会在等待回执处被永久阻塞，由 `run_case` 的超时判失败。
#[test]
fn tokio_scope_b_runtime_driven() {
    assert_b(
        "tokio",
        run_case("tokio｜B 运行时驱动", || {
            let rt = tokio_rt()?;
            let scope = TokioScope::new();
            rt.block_on(scope.run_until(probe_b_runtime_driven(&scope)))
        }),
    );
}

/// 目的：验证 tokio 后端上「`detach` 后存活」用例——`spawn_local` 之后立即
/// `detach()` 的循环任务，在句柄被消费后仍继续被调度直到自行收尾。
///
/// 手段：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_c_detach_survives`；句柄已
/// 消费，循环只能经通道回报自己的求和。
///
/// 判定：回报值等于预期；若 `detach` 实际取消了任务，通道会关闭并判失败。
#[test]
fn tokio_scope_c_detach_survives() {
    assert_c(
        "tokio",
        run_case("tokio｜C detach 后存活", || {
            let rt = tokio_rt()?;
            let scope = TokioScope::new();
            rt.block_on(scope.run_until(probe_c_detach_survives(&scope)))
        }),
    );
}

/// 目的：验证 tokio 后端 `scope.block_on` 便捷阻塞入口同样能驱动本地队列。
///
/// 手段：用多线程运行时（`block_in_place` 的前提）驱动
/// `rt.block_on(async { scope.block_on(..) })`，在其中跑探针 A。
///
/// 判定：交回 `42`；若实现的先决条件不满足会 panic，测试失败。
#[test]
fn tokio_scope_d_block_on_entry() {
    assert_a(
        "tokio",
        run_case("tokio｜D block_on 便捷入口", || {
            let rt = tokio_mt_rt()?;
            let scope = TokioScope::new();
            rt.block_on(async { scope.block_on(probe_a_handle_driven(&scope)) })
        }),
    );
}

// ── compio ────────────────────────────────────────────────────────────────

/// 目的：验证 compio 后端上「句柄驱动」用例（作用域为零大小，队列归运行时所有）。
///
/// 手段：创建 compio 运行时与 `LocalScope`，用 `rt.block_on(scope.run_until(..))`
/// 驱动 `probe_a_handle_driven`（compio 的 `run_until` 即 future 本身）。
///
/// 判定：句柄交回 `42`。
#[test]
fn compio_scope_a_handle_driven() {
    assert_a(
        "compio",
        run_case("compio｜A 句柄驱动", || {
            let rt = compio_rt()?;
            let scope = CompioScope::new();
            rt.block_on(scope.run_until(probe_a_handle_driven(&scope)))
        }),
    );
}

/// 目的：验证 compio 后端上「运行时驱动」用例——宿主不 poll 句柄时循环任务仍持续
/// 消费消息，并在最后一条退出消息后跳出、经句柄交回结果。
///
/// 手段：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_b_runtime_driven`。
///
/// 判定：3 个句柄结果逐条等于预期；超时判失败。
#[test]
fn compio_scope_b_runtime_driven() {
    assert_b(
        "compio",
        run_case("compio｜B 运行时驱动", || {
            let rt = compio_rt()?;
            let scope = CompioScope::new();
            rt.block_on(scope.run_until(probe_b_runtime_driven(&scope)))
        }),
    );
}

/// 目的：验证 compio 后端上「`detach` 后存活」用例。
///
/// 手段：用 `rt.block_on(scope.run_until(..))` 驱动 `probe_c_detach_survives`。
///
/// 判定：循环经通道回报的求和等于预期；若 detach 取消了任务则判失败。
#[test]
fn compio_scope_c_detach_survives() {
    assert_c(
        "compio",
        run_case("compio｜C detach 后存活", || {
            let rt = compio_rt()?;
            let scope = CompioScope::new();
            rt.block_on(scope.run_until(probe_c_detach_survives(&scope)))
        }),
    );
}

/// 目的：验证 compio 后端 `scope.block_on` 便捷阻塞入口同样能驱动本地队列。
///
/// 手段：在 compio 运行时上下文内调用 `scope.block_on(..)`，在其中跑探针 A。
///
/// 判定：交回 `42`。
#[test]
fn compio_scope_d_block_on_entry() {
    assert_a(
        "compio",
        run_case("compio｜D block_on 便捷入口", || {
            let rt = compio_rt()?;
            let scope = CompioScope::new();
            rt.block_on(async { scope.block_on(probe_a_handle_driven(&scope)) })
        }),
    );
}

// ── smol ──────────────────────────────────────────────────────────────────

/// 目的：验证 smol 后端上「句柄驱动」用例——执行器不再挂在 `JoinHandle` 上，
/// 而是归 `LocalScope` 所有。
///
/// 手段：创建 `LocalScope`（内部持有 `LocalExecutor`），用
/// `smol::block_on(scope.run_until(..))` 驱动 `probe_a_handle_driven`。
///
/// 判定：句柄交回 `42`。
#[test]
fn smol_scope_a_handle_driven() {
    assert_a(
        "smol",
        run_case("smol｜A 句柄驱动", || {
            let scope = SmolScope::new();
            smol::block_on(scope.run_until(probe_a_handle_driven(&scope)))
        }),
    );
}

/// 目的：验证 smol 后端上「运行时驱动」用例——**路线 1 之前此格为红**
/// （宿主永久阻塞）。
///
/// 手段：用 `smol::block_on(scope.run_until(..))` 驱动 `probe_b_runtime_driven`；
/// `LocalExecutor::run` 在等待宿主 future 期间持续驱动本地队列，因此循环即使不被
/// poll 也能推进。
///
/// 判定：3 个句柄结果逐条等于预期；若队列仍只能靠 poll 句柄推进，宿主会被永久
/// 阻塞并由 `run_case` 的超时判失败。
#[test]
fn smol_scope_b_runtime_driven() {
    assert_b(
        "smol",
        run_case("smol｜B 运行时驱动", || {
            let scope = SmolScope::new();
            smol::block_on(scope.run_until(probe_b_runtime_driven(&scope)))
        }),
    );
}

/// 目的：验证 smol 后端上「`detach` 后存活」用例——**路线 1 之前此格为红**
/// （任务当场被取消）。执行器归作用域所有后，`detach` 不再连带销毁本地队列。
///
/// 手段：用 `smol::block_on(scope.run_until(..))` 驱动 `probe_c_detach_survives`。
///
/// 判定：循环经通道回报的求和等于预期；若 detach 仍取消任务，通道会关闭并判失败。
#[test]
fn smol_scope_c_detach_survives() {
    assert_c(
        "smol",
        run_case("smol｜C detach 后存活", || {
            let scope = SmolScope::new();
            smol::block_on(scope.run_until(probe_c_detach_survives(&scope)))
        }),
    );
}

/// 目的：验证 smol 后端 `scope.block_on` 便捷阻塞入口同样能驱动本地队列
/// （smol 无任何先决条件，是一条纯便捷入口）。
///
/// 手段：直接调用 `scope.block_on(..)`（内部为 `smol::block_on(ex.run(fut))`），
/// 在其中跑探针 A。
///
/// 判定：交回 `42`。
#[test]
fn smol_scope_d_block_on_entry() {
    assert_a(
        "smol",
        run_case("smol｜D block_on 便捷入口", || {
            let scope = SmolScope::new();
            scope.block_on(probe_a_handle_driven(&scope))
        }),
    );
}
