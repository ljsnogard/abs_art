//! 跨后端 `spawn_local` 行为契约的集成冒烟测试。
//!
//! 本文件里**测试体一份都没有**——测试体全部在 `abs_art_smoke::probe`，且泛型于
//! `Rt: TrSpawnLocal`。本文件只提供三样东西：
//!
//! 1. 三个「创建运行时 + 驱动本地队列」的驱动函数（这是三个后端**唯一**无法
//!    统一的地方，也是本 crate 要量出来的差异所在）；
//! 2. 一个宏，把**逐字相同**的断言逻辑展开到三个后端上，避免三份测试代码
//!    各自漂移（那样就测不出「`abs_art` 的行为是否一致」了）；
//! 3. 每个用例一条独立测试，使得 3×3 矩阵里哪个格子红了可以直接看出来。
//!
//! # 当前基线（本文件建立时）
//!
//! | 用例 | tokio | compio | smol |
//! | --- | --- | --- | --- |
//! | A 句柄驱动 | ✅ | ✅ | ✅ |
//! | B 运行时驱动（不 poll 句柄） | ✅ | ✅ | ❌ 超时 |
//! | C `detach` 后存活 | ✅ | ✅ | ❌ 任务被取消 |
//!
//! smol 的两处失败是**已知缺口**（`abs_art-smol` 目前「每次 `spawn_local` 新建一个
//! `LocalExecutor`，执行器随 `JoinHandle` 存活」），本测试的作用就是把它固定成
//! 一条会红的验收标准。

use std::future::Future;

use abs_art_compio::Runtime as CompioRuntime;
use abs_art_smoke::{
    LOOP_COUNT, LoopResult, expected_sum, probe_a_handle_driven, probe_b_runtime_driven,
    probe_c_detach_survives, run_case,
};
use abs_art_smol::Runtime as SmolRuntime;
use abs_art_tokio::Runtime as TokioRuntime;

/// tokio 的驱动方式：本地队列由 `LocalSet` 持有并驱动，宿主必须跑在
/// `LocalSet::block_on` 里——`tokio::task::spawn_local` 在 `LocalSet` 上下文之外
/// 调用会直接 panic。
fn drive_tokio<F>(f: F) -> F::Output
where
    F: Future,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("创建 tokio current_thread 运行时失败");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

/// compio 的驱动方式：运行时本身就是线程本地的，`rt.block_on` 在等待期间会驱动
/// 它自己的执行器，因此不需要额外的「本地作用域」包装。
fn drive_compio<F>(f: F) -> F::Output
where
    F: Future,
{
    let rt = compio::runtime::Runtime::new().expect("创建 compio 运行时失败");
    rt.block_on(f)
}

/// smol 的驱动方式：`smol::block_on` 只轮询传进去的 future 并驱动 async-io
/// reactor，**不会**驱动 `LocalExecutor`——这正是 `abs_art-smol` 目前必须把执行器
/// 塞进 `JoinHandle` 的原因。
fn drive_smol<F>(f: F) -> F::Output
where
    F: Future,
{
    smol::block_on(f)
}

/// 为一个后端展开三个契约用例。
///
/// 三个后端用的是**同一个宏**展开出的同一份断言逻辑，因此任何后端之间的行为
/// 差异都会直接表现为测试结果差异，而不是测试代码差异。
macro_rules! spawn_local_contract_cases {
    (
        backend: $backend:literal,
        runtime: $rt:ty,
        drive: $drive:ident,
        case_a: $case_a:ident,
        case_b: $case_b:ident,
        case_c: $case_c:ident,
    ) => {
        /// 目的：验证「句柄驱动」用例——`spawn_local` 投递的 `!Send` 任务，在宿主
        /// await 其 `JoinHandle` 时完成，并由句柄把结果交回上层。
        ///
        /// 手段：在独立线程内创建该后端的运行时并进入其本地上下文，调用
        /// `probe_a_handle_driven` 投递一个捕获 `Rc` 的任务，再 await 句柄。
        ///
        /// 判断：句柄返回 `Ok(42)`，且任务内改写的本地状态也是 `42`（两者一致
        /// 才说明结果确实来自那个任务）；超时或返回 `Err` 均判失败。
        #[test]
        fn $case_a() {
            match run_case(concat!($backend, "｜A 句柄驱动"), || {
                $drive(probe_a_handle_driven::<$rt>())
            }) {
                Ok(value) => assert_eq!(
                    value, 42,
                    concat!($backend, "：句柄交回 {}，预期 42"),
                    value
                ),
                Err(e) => panic!("{e}"),
            }
        }

        /// 目的：验证「运行时驱动」用例——若干个消费消息死循环任务，在宿主
        /// **没有 poll 任何 `JoinHandle`** 的情况下仍能持续消费消息、收到最后一条
        /// 退出消息后跳出循环，并把结果经 `JoinHandle` 交回上层。
        ///
        /// 手段：在独立线程内进入该后端的本地上下文，调用
        /// `probe_b_runtime_driven` 投递 3 个循环任务（各自捕获 `Rc`、独占一条
        /// 消息通道），投递正常消息与退出消息，宿主**先**等待循环自己发出的完成
        /// 回执，**之后**才逐个 await 句柄。
        ///
        /// 判断：3 个句柄都返回 `Ok`，结果集合逐条等于 `(循环编号, 预期求和)`；
        /// 若本地队列只能靠 poll 句柄推进，宿主会在等待回执处被永久阻塞，由
        /// `run_case` 的超时判定为失败。
        #[test]
        fn $case_b() {
            match run_case(concat!($backend, "｜B 运行时驱动"), || {
                $drive(probe_b_runtime_driven::<$rt>())
            }) {
                Ok(results) => {
                    let expected: Vec<LoopResult> = (0..LOOP_COUNT)
                        .map(|idx| (idx, expected_sum(idx)))
                        .collect();
                    assert_eq!(results, expected, concat!($backend, "：循环结果与预期不符"));
                }
                Err(e) => panic!("{e}"),
            }
        }

        /// 目的：验证「`detach` 后存活」用例——`spawn_local` 之后**立即**
        /// `detach()` 的循环任务，在句柄被消费后仍继续被调度，直到收到退出消息
        /// 并自行收尾。
        ///
        /// 手段：在独立线程内进入该后端的本地上下文，调用
        /// `probe_c_detach_survives`：投递一个消费消息死循环后立刻 `detach()`，
        /// 随后由宿主投递正常消息与退出消息，再等循环经通道回报自己的求和
        /// （句柄已消费，结果不可能经 `JoinHandle` 交回）。
        ///
        /// 判断：循环在期限内回报的求和等于预期值；若 `detach` 实际取消了任务，
        /// 通道会因发送端被丢弃而关闭，这里会拿到错误并判失败。
        #[test]
        fn $case_c() {
            match run_case(concat!($backend, "｜C detach 后存活"), || {
                $drive(probe_c_detach_survives::<$rt>())
            }) {
                Ok(value) => assert_eq!(
                    value,
                    expected_sum(0),
                    concat!($backend, "：detach 后的循环回报值与预期不符")
                ),
                Err(e) => panic!("{e}"),
            }
        }
    };
}

spawn_local_contract_cases! {
    backend: "tokio",
    runtime: TokioRuntime,
    drive: drive_tokio,
    case_a: tokio_probe_a_handle_driven,
    case_b: tokio_probe_b_runtime_driven,
    case_c: tokio_probe_c_detach_survives,
}

spawn_local_contract_cases! {
    backend: "compio",
    runtime: CompioRuntime,
    drive: drive_compio,
    case_a: compio_probe_a_handle_driven,
    case_b: compio_probe_b_runtime_driven,
    case_c: compio_probe_c_detach_survives,
}

spawn_local_contract_cases! {
    backend: "smol",
    runtime: SmolRuntime,
    drive: drive_smol,
    case_a: smol_probe_a_handle_driven,
    case_b: smol_probe_b_runtime_driven,
    case_c: smol_probe_c_detach_survives,
}
