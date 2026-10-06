//! 跨后端 [`TrTime`] 行为契约的集成冒烟测试。
//!
//! 与 `spawn_local_contract.rs` 同形：**测试体一份都没有**——测试体全部在
//! `abs_art_smoke::time_probe`，且泛型于 `R: TrTime` 并接收运行时**值** `&R`。
//! 本文件只提供三样东西：
//!
//! 1. 每个后端的「创建运行时 + 造运行时值 + 驱动」骨架（创建运行时的代码本来就
//!    属于集成方）；
//! 2. 三组共享的判定——判定在探针里，本文件只把失败原样报出；
//! 3. 每个用例一条独立测试，使得 3 后端 × 8 用例的矩阵里哪个格子红了直接可见。
//!
//! # 驱动形态的统一
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
//! # 为什么用 `rt.block_on(..)`，而不是 `scope.run_until(..)`
//!
//! 两个入口分工不同：
//!
//! | 入口 | 宿主 | 做什么 |
//! | --- | --- | --- |
//! | [`TrBlockOn::block_on`](abs_art::TrBlockOn::block_on) | 运行时**值** | 只等待，不驱动任何本地队列 |
//! | [`TrLocalScope::run_until`](abs_art::TrLocalScope::run_until) | **作用域** | 等待期间驱动本地队列 |
//!
//! 本文件的探针**一个本地任务也不投递**：它们只从运行时值上取计时与时刻。因此这里
//! 既没有队列要驱动，也没有作用域可取——用 `rt.block_on(..)`（只等待）是**正确且
//! 唯一需要**的选择；套一层 `scope.run_until(..)` 会凭空引入一个与计时无关的本地
//! 作用域，反而模糊了「时间来自值」这条分工。（需要本地队列的是第 5 条契约的集成
//! 侧：那里 `value` 与 `scope` 同时出现，用 `run_until` 驱动队列、用 `value` 计时。）
//!
//! # 第 5 条契约：计时能力挂在值上，而不是作用域上
//!
//! 第 5 条（[`tokio_time_capability_comes_from_the_runtime`] 等三格）有三个断言：
//!
//! 1. **类型层面**：`TimeOwner<LocalScope>: TrTime`——`TimeOwner` 泛型于
//!    `S: TrLocalScope`（即任何作用域类型都能代入），却能提供计时能力；这证明
//!    「有作用域」与「能计时」是两件互不依赖的事；
//! 2. **运行期观测**：本地任务（投在作用域上）先睡在**运行时值**的 `delay` 上、
//!    再回报 `7`，而驱动全程只经作用域；
//! 3. 墙上耗时 `>= 2ms`：睡眠确实发生在运行时值的计时器上，没有被跳过。
//!
//! 骨架**逐条内联**在每个用例里，而不是抽成一个收「探针闭包」的公共函数：
//! 探针的返回值会借用传进去的运行时值，而 `FnOnce(&R) -> F` 这种约束表达不了
//! 「返回值与入参借用同期」这层关系（Rust 的闭包不产生 higher-ranked 的返回类型）。
//! 内联的三行骨架反而让「值在哪个上下文里造出来」这件事一眼可见。
//!
//! # 相对早先版本的改动：只有调用形状
//!
//! 更早的探针泛型于**类型** `T: TrTime`，调用点写 `<T as TrTime>::interval(p)`；
//! 值化之后能力挂到运行时**值**上，于是探针收 `&R`、骨架必须在运行时上下文内用
//! `current()` 造值。三个后端的 `TrTime` 能力都挂在各自的 `Runtime<CAPS>` 上
//! （`CAPS` 含 `DELAY` 位），而 `current()` 交出的正是 `Runtime<FULL>`。
//!
//! 七条早期契约的**文档与判定标准都保留**（第 4 条「`now()` 单调不减且与 `delay()`
//! 同源」是 `TrClock` 成为 `TrTime` 超 trait 之后才写得出来的），本轮再加第 5 条。

// `TrLocalScope` 是取作用域（`value.local_scope()`）与驱动作用域
// （`scope.run_until(..)`）的前提，只有第 5 条契约的集成侧用到；其余探针只依赖
// 运行时值的 `TrTime`（各后端经 `current()` 交出的值本身已实现它，不必额外导入）。
use abs_art::TrLocalScope;
use abs_art_smoke::{
    run_case,
    time_probe::{
        probe_clock_is_monotonic_and_shares_delay_source, probe_interval_first_tick_is_immediate,
        probe_interval_is_anchored, probe_sleep_waits_at_least,
        probe_time_capability_comes_from_the_runtime, probe_timeout_elapses,
        probe_timeout_inner_wins, probe_zero_delay_completes_immediately,
        time_owner::{TimeOwner, assert_time_owner_is_trtime},
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

// ── 共享断言：第 5 条契约的「类型层面」那一半 ──────────────────────────────

/// 类型层面断言：作用域类型满足 `TrLocalScope`，而**见证值** `TimeOwner<Scope>`
/// 满足 `TrTime`——计时能力与本地队列是两个互不依赖的类型参数。
///
/// 把这一段放在集成侧（而不是探针里）是故意的：探针只依赖 `abs_art` 的抽象 trait，
/// 这里则需要三个**具体后端的作用域类型**，正好由本文件引入。
fn assert_scope_is_not_the_time_source<Scope_>()
where
    Scope_: TrLocalScope,
    TimeOwner<Scope_>: abs_art::TrTime,
{
    // 见证值只是类型参数占位：它不持有作用域，也不占体积。
    assert_time_owner_is_trtime::<Scope_>();
}

// ── 用例：tokio × 8 条契约 ─────────────────────────────────────────────────

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

/// 目的：验证 tokio 后端上**计时与时刻来自运行时值，而不是作用域**（契约第 5 条）。
///
/// 实施策略：
/// 1. 类型层面：断言 `TimeOwner<abs_art_tokio::LocalScope>: TrTime`——见证类型泛型
///    于「作用域」，因此「有作用域」并不等于「能计时」，计时能力必须另有来源；
/// 2. 运行期：造运行时值 `value` → `value.local_scope()` 取作用域 → 作用域投递一个
///    捕获 `Rc` 的本地任务，任务体内用 `value.delay(2ms)` 睡眠后回报 `7` → 用
///    `scope.run_until` 驱动并量墙上耗时。
///
/// 通过依据：`probe_..._runtime` 返回 `Ok`（回报值 `7` 且耗时 `>= 2ms`）；若计时
/// 从作用域上取用（作用域根本没有 `delay`）本测试无法编译，若睡眠被跳过则耗时判定红。
#[test]
fn tokio_time_capability_comes_from_the_runtime() {
    assert_scope_is_not_the_time_source::<abs_art_tokio::LocalScope>();

    let out = run_case("tokio/time-capability", || {
        let rt = tokio_rt()?;
        rt.block_on(async {
            let value = abs_art_tokio::current();
            let scope = value.local_scope();
            let driven =
                scope.run_until(probe_time_capability_comes_from_the_runtime(&value, &scope));
            driven.await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

// ── 用例：compio × 8 条契约 ────────────────────────────────────────────────

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

/// 目的：验证 compio 后端上**计时与时刻来自运行时值，而不是作用域**（契约第 5 条）。
///
/// 实施策略：
/// 1. 类型层面：断言 `TimeOwner<abs_art_compio::LocalScope>: TrTime`——compio 的作用域
///    钉住运行时实例（`!Send`），但它本身**不**提供 `TrTime`；
/// 2. 运行期：造运行时值 `value` → `value.local_scope()` 取作用域 → 作用域投递本地
///    任务，任务体内用 `value.delay(2ms)` 睡眠后回报 `7` → `scope.run_until` 驱动
///    （compio 侧它即 future 本身，队列由外层 `block_on` tick）。
///
/// 通过依据：探针返回 `Ok`（`7` 且耗时 `>= 2ms`）。
#[test]
fn compio_time_capability_comes_from_the_runtime() {
    assert_scope_is_not_the_time_source::<abs_art_compio::LocalScope>();

    let out = run_case("compio/time-capability", || {
        let rt = compio_rt()?;
        rt.block_on(async {
            let value = abs_art_compio::current();
            let scope = value.local_scope();
            scope
                .run_until(probe_time_capability_comes_from_the_runtime(&value, &scope))
                .await
        })
    });
    assert!(out.is_ok(), "{out:?}");
}

// ── 用例：smol × 8 条契约 ──────────────────────────────────────────────────

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

/// 目的：验证 smol 后端上**计时与时刻来自运行时值，而不是作用域**（契约第 5 条）。
///
/// 实施策略：
/// 1. 类型层面：断言 `TimeOwner<abs_art_smol::LocalScope>: TrTime`——smol 的作用域持
///    自建 `LocalExecutor`，`delay` / `now` 却由运行时值给出；
/// 2. 运行期：值化之后 smol 的运行时值本身是零大小标记，因此这里先用 `current()` 造
///    值、再 `value.local_scope()` 取作用域；作用域投递的本地任务用 `value.delay(2ms)`
///    睡眠后回报 `7`，`smol::block_on(scope.run_until(..))` 驱动全程。
///
/// 通过依据：探针返回 `Ok`（`7` 且耗时 `>= 2ms`）。
#[test]
fn smol_time_capability_comes_from_the_runtime() {
    assert_scope_is_not_the_time_source::<abs_art_smol::LocalScope>();

    let out = run_case("smol/time-capability", || {
        let value = abs_art_smol::current();
        let scope = value.local_scope();
        smol::block_on(
            scope.run_until(probe_time_capability_comes_from_the_runtime(&value, &scope)),
        )
    });
    assert!(out.is_ok(), "{out:?}");
}
