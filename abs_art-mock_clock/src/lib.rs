//! 手动（虚拟）时钟：把「等待」从真实时间上摘下来，交给测试自己推进。
//!
//! # 这个 crate 解决什么
//!
//! tokio 的 `test-util` 提供「可暂停的时钟」：`Instant::now()` 读虚拟时刻、`sleep` 注册
//! 进虚拟时间、runtime 空闲时自动推进——于是 `#[tokio::test(start_paused = true)]` 下
//! `sleep(1h)` 瞬间完成。**compio / smol 没有这个能力**，于是「带虚拟时间的测试」只能
//! 写在 tokio 上。
//!
//! 本 crate 把这项能力做成**与后端无关**的一份实现：一个共享的手动时钟
//! （[`ManualClock`]）、一个由它驱动的 delay（[`MockDelay`]）、一个把任意运行时值的
//! 「时间」替换成手动时钟的装饰器（[`ManualTime`]），以及一个统一驱动
//! （[`Supervisor`]）。三个后端各自以**可选 feature** 接入驱动胶水。
//!
//! # 概念归属：能力在 `abs_art`，实现在这里
//!
//! **能力 trait 在基础 crate 里**：`abs_art::TrClock`（能读时刻）与
//! `abs_art::TrMockClock`（时刻**可以被调用方推进**——它是 `TrClock` 的**特例**/超 trait）。
//! 业务代码只约束 `TrClock`，测试代码才约束 `TrMockClock`，因此同一份业务代码在真实
//! 时间与虚拟时间下零改动。
//!
//! **本 crate 提供实现**：手动时钟状态（[`ManualClock`]）、到期唤醒表、delay
//! （[`MockDelay`]）、装饰器（[`ManualTime`]）与统一驱动（[`Supervisor`]）。
//! 三个扩展点也在这里：[`ManualClockApi`]（换共享状态/机制）、[`MockInstant`]（换时刻
//! 类型）、以及驱动钩子（闭包）。第三方若要另一套实现，实现这几个 trait 即可。
//!
//! # 用法（以 smol 为例）
//!
//! ```ignore
//! use abs_art::TrMockClock;
//! use abs_art_mock_clock::{ManualClock, ManualTime};
//!
//! let clock = ManualClock::new();
//! let scope = abs_art_smol::current().local_scope();
//! let value = ManualTime::new(abs_art_smol::current(), clock.clone());
//!
//! scope.block_on_advancing(&clock, async {
//!     let started = value.now();
//!     // 测试也可以自己推：`advance` 是 future（与 `tokio::time::advance` 同形）
//!     value.advance(core::time::Duration::from_secs(1800)).await;
//!     value.delay(core::time::Duration::from_secs(1800)).await;
//!     assert_eq!(value.now() - started, core::time::Duration::from_secs(3600));
//! });
//! ```
//!
//! 剩下的时间由[`Supervisor`]在「没有别的活可干」时推进，真实耗时几乎为零。
//!
//! # 为什么驱动钩子是闭包而不是 trait
//!
//! 各后端的「执行器 tick」钩子（smol 的 `LocalExecutor::try_tick`、compio 的
//! `Runtime::run`）没法用 trait 表达：若本 crate 定义 `trait Tick`，后端 crate 就
//! 必须为 `smol::LocalExecutor` 这类**外来类型**实现它——外来 trait + 外来类型 =
//! `E0117`（实测）。因此 [`Supervisor`] 收一个 `Fn() -> bool` 闭包；tokio 没有可用的
//! tick 钩子，传一个恒为 `false` 的闭包即可（它的 `block_on` 自己会驱动任务）。
//!
//! # 平台
//!
//! 本 crate 是 **`no_std`** 的：只用 `core` + `alloc`（`alloc::sync::Arc` 共享时钟、
//! `alloc::vec::Vec` 存到期表），互斥用同仓库的 [`atomic_sync`]（`no_std` 的
//! `SpinningMutexOwned`）而不是自己造锁。模拟时钟只依赖 `abs_art` 与 `atomic_sync`；
//! 运行时是否实质上依赖 std，与本 crate 无关——三个后端各自按需选用本 crate 即可。

#![no_std]
#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod advance;
pub mod clock;
pub mod decorator;
pub mod delay;
pub mod driver;
pub mod instant;

#[cfg(test)]
mod support_;

pub use abs_art::TrMockClock;
pub use advance::MockAdvance;
pub use clock::{ManualClock, ManualClockApi};
pub use decorator::ManualTime;
pub use delay::{MockDelay, MockInterval};
pub use driver::Supervisor;
pub use instant::{MillisInstant, MockInstant};
