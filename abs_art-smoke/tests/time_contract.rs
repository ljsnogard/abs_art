//! 跨后端 [`TrTime`] 行为契约的集成冒烟测试（v0.4：值化的运行时）。
//!
//! 与 `spawn_local_contract.rs` 同形：**测试体一份都没有**——测试体全部在
//! `abs_art_smoke::time_probe`，且泛型于 `R: TrTime` 并接收运行时**值** `&R`。
//! 本文件只提供三样东西：
//!
//! 1. 每个后端的「创建运行时 + 造运行时值 + 驱动」骨架（创建运行时的代码本来就
//!    属于集成方）；
//! 2. 三组共享的判定——判定在探针里，本文件只把失败原样报出；
//! 3. 每个用例一条独立测试，使得 3 后端 × 7 用例的矩阵里哪个格子红了直接可见。
//!
//! # 驱动形态的统一（v0.4）
//!
//! ```text
//! OUTER_DRIVER {
//!     let value = abs_art_<backend>::current();   // 运行时值在上下文内造出
//!     probe(&value).await                          // 计时器与时刻源都来自这个值
//! }
//!   ├─ tokio  : rt.block_on(..)      —— current_thread + enable_all（含 time 驱动）
//!   ├─ compio : rt.block_on(..)
//!   └─ smol   : smol::block_on(..)   —— async-io 的全局反应器
//! ```
//!
//! 骨架**逐条内联**在每个用例里，而不是抽成一个收「探针闭包」的公共函数：
//! 探针的返回值会借用传进去的运行时值，而 `FnOnce(&R) -> F` 这种约束表达不了
//! 「返回值与入参借用同期」这层关系（Rust 的闭包不产生 higher-ranked 的返回类型）。
//! 内联的三行骨架反而让「值在哪个上下文里造出来」这件事一眼可见。
//!
//! # 相对 v0.3 的改动：只有调用形状
//!
//! v0.3 的探针泛型于**类型** `T: TrTime`，调用点写 `<T as TrTime>::interval(p)`；
//! v0.4 把能力挂到运行时**值**上，于是探针收 `&R`、骨架必须在运行时上下文内用
//! `current()` 造值。三个后端的 `TrTime` 能力都挂在各自的 `Runtime<CAPS>` 上
//! （`CAPS` 含 `DELAY` 位），而 `current()` 交出的正是 `Runtime<FULL>`。
//!
//! 六条契约的**文档与判定标准都保留**，另外新增第 4 条「`now()` 单调不减且与
//! `delay()` 同源」——它是 `TrClock` 成为 `TrTime` 超 trait 之后才写得出来的契约。

use abs_art_smoke::{
    run_case,
    time_probe::{
        probe_clock_is_monotonic_and_shares_delay_source, probe_interval_first_tick_is_immediate,
        probe_interval_is_anchored, probe_sleep_waits_at_least, probe_timeout_elapses,
        probe_timeout_inner_wins, probe_zero_delay_completes_immediately,
    },
};

// ── 运行时的创建（集成方职责，三个后端各不相同）────────────────────────────

/// 创建 tokio 单线程运行时。
///
/// `current_thread` + `enable_all`：契约用例不需要多线程，但需要 time 驱动。
fn tokio_rt() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

/// 创建 compio 运行时。
fn compio_rt() -> Result<compio::runtime::Runtime, String> {
    compio::runtime::Runtime::new().map_err(|e| e.to_string())
}

// ── 用例：tokio × 7 条契约 ─────────────────────────────────────────────────

/// 目的：验证 tokio 后端上 `delay` 不早于 duration 返回（契约第 1 条）。
///
/// 实施策略：经 `tokio_rt` 建真实 tokio 运行时，在运行时上下文内用
/// `abs_art_tokio::current()` 造运行时值，跑 `probe_sleep_waits_at_least`。
///
/// 通过依据：探针返回 `Ok`（否则给出实际耗时）。
#[test]
fn tokio_sleep_waits_at_least() {
    let out = run_case("tokio/sleep", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            probe_sleep_waits_at_least(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上 `delay(0)` 立即就绪（契约第 1 条补充）。
///
/// 实施策略：同上骨架，跑 `probe_zero_delay_completes_immediately`。
///
/// 通过依据：探针返回 `Ok`（耗时 < 100 ms）。
#[test]
fn tokio_zero_delay_completes_immediately() {
    let out = run_case("tokio/zero-delay", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            probe_zero_delay_completes_immediately(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上周期源首次 tick 立即完成（契约第 2 条）。
///
/// 实施策略：同上骨架，跑 `probe_interval_first_tick_is_immediate`。
///
/// 通过依据：探针返回 `Ok`；若首次被推迟一个 5 秒周期，用例会被 `run_case` 记超时。
#[test]
fn tokio_interval_first_tick_is_immediate() {
    let out = run_case("tokio/interval-first", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            probe_interval_first_tick_is_immediate(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上周期源锚定在构造时刻（契约第 3 条）。
///
/// 实施策略：同上骨架，跑 `probe_interval_is_anchored`。
///
/// 通过依据：探针返回 `Ok`；退化成「每响之后再等一个周期」则判失败。
#[test]
fn tokio_interval_is_anchored() {
    let out = run_case("tokio/interval-anchored", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            probe_interval_is_anchored(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上 `now()` 单调不减，且与 `delay()` 同源（契约第 4 条）。
///
/// 实施策略：同上骨架，跑 `probe_clock_is_monotonic_and_shares_delay_source`；
/// 探针在同**一个运行时值**上连读 `now()` 并 `delay(30ms)` 后复查。
///
/// 通过依据：探针返回 `Ok`。tokio 后端的 `Instant` 取 `tokio::time::Instant`，与它
/// 的 time 驱动同一基准——若实现改成读 `std::time::Instant`，`test-util` 的
/// 虚拟时间下就会露馅（本用例是那条约束在契约矩阵里的探针）。
#[test]
fn tokio_clock_is_monotonic_and_shares_delay_source() {
    let out = run_case("tokio/clock", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            probe_clock_is_monotonic_and_shares_delay_source(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 tokio 后端上 `timeout` 在内层先完成时返回输出（契约第 6 条）。
///
/// 实施策略：同上骨架，跑 `probe_timeout_inner_wins`。
///
/// 通过依据：探针返回 `Ok(7)`。
#[test]
fn tokio_timeout_inner_wins() {
    let out = run_case("tokio/timeout-inner", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            probe_timeout_inner_wins(&value).await
        })
    });
    assert_eq!(out, Ok(7u8));
}

/// 目的：验证 tokio 后端上期限先到时 `timeout` 返回统一的 `Elapsed`（契约第 6 条）。
///
/// 实施策略：同上骨架，跑 `probe_timeout_elapses`。
///
/// 通过依据：探针返回 `Ok`（含「不早于期限」与文案「期限已到」两条判定）。
#[test]
fn tokio_timeout_elapses() {
    let out = run_case("tokio/timeout-elapses", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            probe_timeout_elapses(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

// ── 用例：compio × 7 条契约 ────────────────────────────────────────────────

/// 目的：验证 compio 后端上 `delay` 不早于 duration 返回（契约第 1 条）。
///
/// 实施策略：经 `compio_rt` 建真实 compio 运行时，在运行时上下文内用
/// `abs_art_compio::current()` 造运行时值，跑同一份探针。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_sleep_waits_at_least() {
    let out = run_case("compio/sleep", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            probe_sleep_waits_at_least(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上 `delay(0)` 立即就绪（契约第 1 条补充）。
///
/// 实施策略：同上骨架，跑 `probe_zero_delay_completes_immediately`。
///
/// 通过依据：探针返回 `Ok`（耗时 < 100 ms）。
#[test]
fn compio_zero_delay_completes_immediately() {
    let out = run_case("compio/zero-delay", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            probe_zero_delay_completes_immediately(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上周期源首次 tick 立即完成（契约第 2 条）。
///
/// 实施策略：同上骨架，跑 `probe_interval_first_tick_is_immediate`。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_interval_first_tick_is_immediate() {
    let out = run_case("compio/interval-first", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            probe_interval_first_tick_is_immediate(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上周期源锚定在构造时刻（契约第 3 条）。
///
/// 实施策略：同上骨架，跑 `probe_interval_is_anchored`。compio 的相位对齐正是本
/// 契约的原型，此格是其余两格的参照。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_interval_is_anchored() {
    let out = run_case("compio/interval-anchored", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            probe_interval_is_anchored(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上 `now()` 单调不减，且与 `delay()` 同源（契约第 4 条）。
///
/// 实施策略：同上骨架，跑同一份时钟探针。
///
/// 通过依据：探针返回 `Ok`；compio 后端给 `std::time::Instant`，而它的计时器也落在
/// 同一时间基准上，两条判定都应满足。
#[test]
fn compio_clock_is_monotonic_and_shares_delay_source() {
    let out = run_case("compio/clock", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            probe_clock_is_monotonic_and_shares_delay_source(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 compio 后端上 `timeout` 在内层先完成时返回输出（契约第 6 条）。
///
/// 实施策略：同上骨架，跑 `probe_timeout_inner_wins`。
///
/// 通过依据：探针返回 `Ok(7)`。
#[test]
fn compio_timeout_inner_wins() {
    let out = run_case("compio/timeout-inner", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            probe_timeout_inner_wins(&value).await
        })
    });
    assert_eq!(out, Ok(7u8));
}

/// 目的：验证 compio 后端上期限先到时 `timeout` 返回统一的 `Elapsed`（契约第 6 条）。
///
/// 实施策略：同上骨架，跑 `probe_timeout_elapses`。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn compio_timeout_elapses() {
    let out = run_case("compio/timeout-elapses", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            probe_timeout_elapses(&value).await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

// ── 用例：smol × 7 条契约 ──────────────────────────────────────────────────

/// 目的：验证 smol 后端上 `delay` 不早于 duration 返回（契约第 1 条）。
///
/// 实施策略：用 `abs_art_smol::current()` 造运行时值，在 async-io 的全局反应器上
/// （`smol::block_on`）跑同一份探针。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_sleep_waits_at_least() {
    let out = run_case("smol/sleep", || {
        let value = abs_art_smol::current();
        smol::block_on(probe_sleep_waits_at_least(&value))
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上 `delay(0)` 立即就绪（契约第 1 条补充）。
///
/// 实施策略：同上骨架，跑 `probe_zero_delay_completes_immediately`。
///
/// 通过依据：探针返回 `Ok`（耗时 < 100 ms）。
#[test]
fn smol_zero_delay_completes_immediately() {
    let out = run_case("smol/zero-delay", || {
        let value = abs_art_smol::current();
        smol::block_on(probe_zero_delay_completes_immediately(&value))
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上周期源首次 tick 立即完成（契约第 2 条）。
///
/// 实施策略：同上骨架，跑 `probe_interval_first_tick_is_immediate`。这一格最
/// 关键：`async_io::Timer::interval` 的原生首次语义是「一个周期之后」，本后端为此
/// 自建了周期源。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_interval_first_tick_is_immediate() {
    let out = run_case("smol/interval-first", || {
        let value = abs_art_smol::current();
        smol::block_on(probe_interval_first_tick_is_immediate(&value))
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上周期源锚定在构造时刻（契约第 3 条）。
///
/// 实施策略：同上骨架，跑 `probe_interval_is_anchored`——自建实现靠
/// `smol::Timer::interval` 的内部锚定，此格验证它没有被写成「每响之后再等」。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_interval_is_anchored() {
    let out = run_case("smol/interval-anchored", || {
        let value = abs_art_smol::current();
        smol::block_on(probe_interval_is_anchored(&value))
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上 `now()` 单调不减，且与 `delay()` 同源（契约第 4 条）。
///
/// 实施策略：同上骨架，跑同一份时钟探针。
///
/// 通过依据：探针返回 `Ok`；smol 后端给 `std::time::Instant`，与 `smol::Timer`
/// 同一时间基准。
#[test]
fn smol_clock_is_monotonic_and_shares_delay_source() {
    let out = run_case("smol/clock", || {
        let value = abs_art_smol::current();
        smol::block_on(probe_clock_is_monotonic_and_shares_delay_source(&value))
    });
    assert!(out.is_ok(), "{out:?}");
}

/// 目的：验证 smol 后端上 `timeout` 在内层先完成时返回输出（契约第 6 条）。
///
/// 实施策略：同上骨架，跑 `probe_timeout_inner_wins`。
///
/// 通过依据：探针返回 `Ok(7)`。
#[test]
fn smol_timeout_inner_wins() {
    let out = run_case("smol/timeout-inner", || {
        let value = abs_art_smol::current();
        smol::block_on(probe_timeout_inner_wins(&value))
    });
    assert_eq!(out, Ok(7u8));
}

/// 目的：验证 smol 后端上期限先到时 `timeout` 返回统一的 `Elapsed`（契约第 6 条）。
///
/// 实施策略：同上骨架，跑 `probe_timeout_elapses`。
///
/// 通过依据：探针返回 `Ok`。
#[test]
fn smol_timeout_elapses() {
    let out = run_case("smol/timeout-elapses", || {
        let value = abs_art_smol::current();
        smol::block_on(probe_timeout_elapses(&value))
    });
    assert!(out.is_ok(), "{out:?}");
}
