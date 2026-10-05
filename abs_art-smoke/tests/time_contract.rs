//! 跨后端 [`TrTime`] 行为契约的集成冒烟测试。
//!
//! 与 `spawn_local_contract.rs` 同形：**测试体一份都没有**——测试体全部在
//! `abs_art_smoke::time_probe`，且泛型于 `T: TrTime`。本文件只提供三样东西：
//!
//! 1. 每个后端的「创建运行时 + 驱动」骨架（创建运行时的代码本来就属于集成方）；
//! 2. 三组共享的判定——判定在探针里，本文件只把失败原样报出；
//! 3. 每个用例一条独立测试，使得 3 后端 × 5 用例的矩阵里哪个格子红了直接可见。
//!
//! # 驱动形态的统一
//!
//! ```text
//! OUTER_DRIVER(probe::<BackendRuntime>())
//!   ├─ tokio  : rt.block_on(..)      —— current_thread + enable_all（含 time 驱动）
//!   ├─ compio : rt.block_on(..)
//!   └─ smol   : smol::block_on(..)   —— async-io 的全局反应器
//! ```
//!
//! 三个后端的 `TrTime` 能力都挂在各自的 `Runtime<CAPS>` 上（`CAPS` 含
//! `DELAY` 位），因此这里一律用 `Runtime<{ FULL }>`。

use core::future::Future;

use abs_art::FULL;
use abs_art_compio::Runtime as CompioRuntime;
use abs_art_smoke::{
    run_case,
    time_probe::{
        probe_interval_first_tick_is_immediate, probe_interval_is_anchored,
        probe_sleep_waits_at_least, probe_timeout_elapses, probe_timeout_inner_wins,
        probe_zero_delay_completes_immediately,
    },
};
use abs_art_smol::Runtime as SmolRuntime;
use abs_art_tokio::Runtime as TokioRuntime;

// ── 运行时的创建与驱动（集成方职责，三个后端各不相同）──────────────────────

/// 在 tokio 运行时里跑一个探针。
///
/// `current_thread` + `enable_all`：契约用例不需要多线程，但需要 time 驱动。
fn tokio_case<T, F>(label: &str, probe: impl FnOnce() -> F + Send + 'static) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + 'static,
{
    run_case(label, move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        rt.block_on(probe())
    })
}

/// 在 compio 运行时里跑一个探针。
fn compio_case<T, F>(label: &str, probe: impl FnOnce() -> F + Send + 'static) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + 'static,
{
    run_case(label, move || {
        let rt = compio::runtime::Runtime::new().map_err(|e| e.to_string())?;
        rt.block_on(probe())
    })
}

/// 在 smol 的全局执行器上跑一个探针。
fn smol_case<T, F>(label: &str, probe: impl FnOnce() -> F + Send + 'static) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = Result<T, String>> + 'static,
{
    run_case(label, move || smol::block_on(probe()))
}

// ── 用例：3 后端 × 5 条契约 ────────────────────────────────────────────────

/// 目的：验证 tokio 后端上 `sleep` 不早于 duration 返回（契约第 1 条）。
///
/// 实施策略：经 `tokio_case` 在真实 tokio 运行时里跑 `probe_sleep_waits_at_least`。
///
/// 通过依据：探针返回 `Ok`（否则给出实际耗时）。
#[test]
fn tokio_sleep_waits_at_least() {
    let out = tokio_case("tokio/sleep", || {
        probe_sleep_waits_at_least::<TokioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上周期源首次 tick 立即完成（契约第 2 条）。
///
/// 实施策略：经 `tokio_case` 跑 `probe_interval_first_tick_is_immediate`。
///
/// 通过依据：探针返回 `Ok`；若首次被推迟一个 5 秒周期，用例会被 `run_case` 记超时。
#[test]
fn tokio_interval_first_tick_is_immediate() {
    let out = tokio_case("tokio/interval-first", || {
        probe_interval_first_tick_is_immediate::<TokioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上周期源锚定在构造时刻（契约第 3 条）。
///
/// 实施策略：经 `tokio_case` 跑 `probe_interval_is_anchored`。
///
/// 通过依据：探针返回 `Ok`；退化成「每响之后再等一个周期」则判失败。
#[test]
fn tokio_interval_is_anchored() {
    let out = tokio_case("tokio/interval-anchored", || {
        probe_interval_is_anchored::<TokioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上 `timeout` 在内层先完成时返回输出（契约第 6 条）。
///
/// 实施策略：经 `tokio_case` 跑 `probe_timeout_inner_wins`。
///
/// 通过依据：探针返回 `Ok(7)`。
#[test]
fn tokio_timeout_inner_wins() {
    let out = tokio_case("tokio/timeout-inner", || {
        probe_timeout_inner_wins::<TokioRuntime<{ FULL }>>()
    });
    assert_eq!(out, Ok(7u8));
}

/// 目的：验证 tokio 后端上期限先到时 `timeout` 返回统一的 `Elapsed`（契约第 6 条）。
///
/// 实施策略：经 `tokio_case` 跑 `probe_timeout_elapses`。
///
/// 通过依据：探针返回 `Ok`（含「不早于期限」与文案「期限已到」两条判定）。
#[test]
fn tokio_timeout_elapses() {
    let out = tokio_case("tokio/timeout-elapses", || {
        probe_timeout_elapses::<TokioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上 `sleep` 不早于 duration 返回（契约第 1 条）。
///
/// 实施策略：经 `compio_case` 在真实 compio 运行时里跑同一份探针。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_sleep_waits_at_least() {
    let out = compio_case("compio/sleep", || {
        probe_sleep_waits_at_least::<CompioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上周期源首次 tick 立即完成（契约第 2 条）。
///
/// 实施策略：经 `compio_case` 跑 `probe_interval_first_tick_is_immediate`。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_interval_first_tick_is_immediate() {
    let out = compio_case("compio/interval-first", || {
        probe_interval_first_tick_is_immediate::<CompioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上周期源锚定在构造时刻（契约第 3 条）。
///
/// 实施策略：经 `compio_case` 跑 `probe_interval_is_anchored`。compio 的相位对齐
/// 正是本契约的原型，此格是其余两格的参照。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_interval_is_anchored() {
    let out = compio_case("compio/interval-anchored", || {
        probe_interval_is_anchored::<CompioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上 `timeout` 在内层先完成时返回输出（契约第 6 条）。
///
/// 实施策略：经 `compio_case` 跑 `probe_timeout_inner_wins`。
///
/// 通过依据：探针返回 `Ok(7)`。
#[test]
fn compio_timeout_inner_wins() {
    let out = compio_case("compio/timeout-inner", || {
        probe_timeout_inner_wins::<CompioRuntime<{ FULL }>>()
    });
    assert_eq!(out, Ok(7u8));
}

/// 目的：验证 compio 后端上期限先到时 `timeout` 返回统一的 `Elapsed`（契约第 6 条）。
///
/// 实施策略：经 `compio_case` 跑 `probe_timeout_elapses`。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_timeout_elapses() {
    let out = compio_case("compio/timeout-elapses", || {
        probe_timeout_elapses::<CompioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上 `sleep` 不早于 duration 返回（契约第 1 条）。
///
/// 实施策略：经 `smol_case` 在 async-io 的全局反应器上跑同一份探针。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_sleep_waits_at_least() {
    let out = smol_case("smol/sleep", || {
        probe_sleep_waits_at_least::<SmolRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上周期源首次 tick 立即完成（契约第 2 条）。
///
/// 实施策略：经 `smol_case` 跑 `probe_interval_first_tick_is_immediate`。这一格最
/// 关键：`async_io::Timer::interval` 的原生首次语义是「一个周期之后」，本后端为此
/// 自建了周期源。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_interval_first_tick_is_immediate() {
    let out = smol_case("smol/interval-first", || {
        probe_interval_first_tick_is_immediate::<SmolRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上周期源锚定在构造时刻（契约第 3 条）。
///
/// 实施策略：经 `smol_case` 跑 `probe_interval_is_anchored`——自建实现靠
/// `smol::Timer::interval` 的内部锚定，此格验证它没有被写成「每响之后再等」。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_interval_is_anchored() {
    let out = smol_case("smol/interval-anchored", || {
        probe_interval_is_anchored::<SmolRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上 `timeout` 在内层先完成时返回输出（契约第 6 条）。
///
/// 实施策略：经 `smol_case` 跑 `probe_timeout_inner_wins`。
///
/// 通过依据：探针返回 `Ok(7)`。
#[test]
fn smol_timeout_inner_wins() {
    let out = smol_case("smol/timeout-inner", || {
        probe_timeout_inner_wins::<SmolRuntime<{ FULL }>>()
    });
    assert_eq!(out, Ok(7u8));
}

/// 目的：验证 smol 后端上期限先到时 `timeout` 返回统一的 `Elapsed`（契约第 6 条）。
///
/// 实施策略：经 `smol_case` 跑 `probe_timeout_elapses`。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_timeout_elapses() {
    let out = smol_case("smol/timeout-elapses", || {
        probe_timeout_elapses::<SmolRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上 `delay(0)` 立即就绪（契约第 1 条补充）。
///
/// 实施策略：经 `tokio_case` 在真实 tokio 运行时里跑 `probe_zero_delay_completes_immediately`。
///
/// 通过依据：探针返回 `Ok`（耗时 < 100 ms）。
#[test]
fn tokio_zero_delay_completes_immediately() {
    let out = tokio_case("tokio/zero-delay", || {
        probe_zero_delay_completes_immediately::<TokioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上 `delay(0)` 立即就绪（契约第 1 条补充）。
///
/// 实施策略：经 `compio_case` 在真实 compio 运行时里跑 `probe_zero_delay_completes_immediately`。
///
/// 通过依据：探针返回 `Ok`（耗时 < 100 ms）。
#[test]
fn compio_zero_delay_completes_immediately() {
    let out = compio_case("compio/zero-delay", || {
        probe_zero_delay_completes_immediately::<CompioRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上 `delay(0)` 立即就绪（契约第 1 条补充）。
///
/// 实施策略：经 `smol_case` 在真实 smol 运行时里跑 `probe_zero_delay_completes_immediately`。
///
/// 通过依据：探针返回 `Ok`（耗时 < 100 ms）。
#[test]
fn smol_zero_delay_completes_immediately() {
    let out = smol_case("smol/zero-delay", || {
        probe_zero_delay_completes_immediately::<SmolRuntime<{ FULL }>>()
    });
    assert!(out.is_ok(), "{out:?}");
}
