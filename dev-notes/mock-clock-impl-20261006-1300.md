# mock clock 落地：独立的 `abs_art-mock_clock` + 三后端可选 feature

日期：2026-10-06 13:00
分支：`feat/abs_art-runtime`
性质：**实施记录**（新 crate + 三后端 feature 已落地并全绿）
前置：`mock-clock-driver-probe-20261006-1215.md`（驱动策略可行性）、
`instant-trait-and-manual-clock-20261006-1140.md`（概念拆分的讨论）

---

## 0. 一句话

「虚拟（手动）时钟」成为**独立 crate**：一份与后端无关的实现（手动时刻状态 + 到期唤醒表 +
delay + 装饰器 + 统一驱动），三个后端各以**可选 feature `mock-clock`** 接入驱动胶水。
`abs_art` 里**没有**任何测试设施。

## 1. 落地形状

```text
abs_art-mock_clock/
  src/instant.rs    MockInstant（扩展点）+ MillisInstant（内建，毫秒刻度）
  src/clock.rs      ManualClockApi（扩展点）+ ManualClock（Arc + 私有自旋锁，到期唤醒表）
  src/advance.rs    MockAdvance（TrMockClock 的 GAT 返回类型）
  src/delay.rs      MockDelay（TrDelay::Delay）+ MockInterval（TrTime::Interval）
  src/decorator.rs  TrMockClock（扩展点）+ ManualTime<R, C>（装饰器）
  src/driver.rs     Supervisor（「空闲即推进」驱动 + 停滞即 panic）
```

各后端（`abs_art-tokio` / `abs_art-compio` / `abs_art-smol`）新增：

```toml
mock-clock = ["dep:abs_art-mock_clock", "local_scope"]
```

以及一个方法（三后端同名同形，挂在 `LocalScope` 上）：

```rust
pub fn block_on_advancing<F, C>(&self, clock: &C, body: F) -> F::Output
where F: Future, C: abs_art_mock_clock::ManualClockApi
```

## 2. 落地时做的决定（以及理由）

| 决定 | 理由 |
| --- | --- |
| `ManualClock` 用 `Arc` + **`atomic_sync` 的 `SpinningMutexOwned`** | 时钟能跨线程共享（`Send + Sync`），同时不依赖 std——见 §7 |
| 本 crate 是 **`no_std`**（`core` + `alloc`） | 模拟时钟只依赖 `abs_art`；运行时是否依赖 std 与本 crate 无关——见 §7 |
| 时刻类型内建 `MillisInstant`，把 `MockInstant` 留作扩展点 | 不把 `embedded-timers` 拉成依赖（本地 checkout、且会带进 `embedded-hal`/`nb`/`void`）；第三方（含 `Instant64<FREQ>`）实现 `MockInstant` 即可换时刻类型 |
| tick 钩子是**闭包** `Fn() -> bool`，不是 trait | 后端 crate 无法为 `smol::LocalExecutor` / `compio::runtime::Runtime` 这类外来类型实现本 crate 的 trait（`E0117`，探针实测） |
| 驱动挂在 `LocalScope` 上（而不是 `Runtime` 上） | 三后端的作用域都能拿到自己的执行器（tokio 的 `LocalSet`、compio 的 `Runtime`、smol 的 `LocalExecutor`），形状统一；而且「驱动队列」本来就是作用域的职责 |
| `TrMockClock` 定义在 **`abs_art`**（`TrClock` 的超 trait） | 它是「时刻可以被调用方推进」这一**能力**，与 `TrClock` 同族；按讨论修正：能力进 `abs_art`，只有**实现**才留在新 crate |
| 不为 mock 开能力位 | 「声明」的落点是**构造**：mock 值只能由测试显式构造（`ManualTime::new`），不存在静默选错的风险 |
| 驱动在停滞时 **panic** | 「既没有就绪任务、也没有可推进的定时器」= 真死锁（忘了推进 / 时钟被冻结 / 等一个永不到来的事件）；静默挂起比 panic 难查得多 |
| 冻结（`pause()` / `set_frozen(true)`）时驱动**不**自动推进 | 保留 tokio `pause()` 那种「测试自己控制时间」的用法；此时由主体自己 `advance` |

## 3. 三后端的驱动差异（实测）

| 后端 | `block_on_advancing` 实现 | tick 钩子 |
| --- | --- | --- |
| smol | `smol::block_on(Supervisor::new(body, clock, \|\| ticker.try_tick()))` | `LocalExecutor::try_tick()` |
| compio | `rt.block_on(Supervisor::new(body, clock, move \|\| ticker.run()))` | `Runtime::run()` |
| tokio | 上下文外 `handle.block_on(local.run_until(..))`；上下文内 `block_in_place` 包一层 | 不需要（`\|\| false`） |

## 4. 证据（全部实跑）

`just test-mock-clock` → **EXIT=0**：

```text
abs_art-mock_clock   19 passed（+ 3 doctests）
abs_art-tokio        23 passed（+ 8 doctests + 1）= 21 + 2 条虚拟时间用例
abs_art-compio       28 passed（+ 13 + 4）      = 26 + 2 条虚拟时间用例
abs_art-smol         34 passed（+ 8 + 1）       = 32 + 2 条虚拟时间用例
```

每个后端的两条用例：

1. `virtual_hour_passes_instantly`：`delay(1h)` 在真实耗时 < 1s 内完成，虚拟时刻恰为 3600_000ms；
2. `spawned_local_task_runs_on_virtual_time`：**投递到本地队列**的任务也跑在虚拟时间上
   （`spawn_local` → `block_on_advancing` 驱动 → 取回 42，虚拟时刻 1800_000ms，真实 < 1s）。

其它：

- `cargo test --workspace` → 23 个目标全 ok、0 失败（`mock-clock` 不在默认 features 里，
  默认构建不受影响）；
- `cargo clippy -p abs_art-mock_clock --all-targets` 与三后端 `--features mock-clock` → 各 crate 0 诊断；
- `cargo doc -p abs_art --no-deps` → 0 断链。

## 5. 留作后续

1. **smoke 契约**：可以再加一条「虚拟时间契约」（3 后端 × 1 格），把「同一份业务代码跑虚拟
   时间」也纳入契约矩阵；目前证据是各后端的单元测试。
2. **`embedded-timers` 集成**：为 `Instant64<FREQ>` 提供一个可选 feature 的 `MockInstant` 实现。
3. **no_std 版本**：把共享状态换成 `alloc` + 自旋锁（`ManualClockApi` 已留好扩展点）。
4. **`CLOCK` 能力位**：与「时钟是否要独立能力位」那条待裁决绑在一起，本轮未动。
"""

---

## 6. 同日修正（讨论后的三处改动）

### 6.1 `TrMockClock` 移进 `abs_art`，作为 `TrClock` 的**特例**

我最初的表述「mock clock 概念不进 `abs_art`」不准确。正确的分工是：

| | 在哪 | 内容 |
| --- | --- | --- |
| **能力** | `abs_art` | `TrClock`（能读时刻）+ `TrMockClock: TrClock`（时刻**可被调用方推进**） |
| **实现** | `abs_art-mock_clock` | 手动时钟状态、到期唤醒表、`MockDelay`、`ManualTime`、`Supervisor` |

`TrMockClock` 是 `TrClock` 的超 trait（特例），不是并列的第二种时钟——理由见
`instant-trait-and-manual-clock-20261006-1140.md` §3.2（并列会重新制造「两个时钟源」的
错配；Rust 也表达不了 `!TrMockClock`）。于是业务代码只约束 `TrClock`，测试代码才约束
`TrMockClock`，同一份业务代码在真实/虚拟时间下零改动。

新 crate 的机制 trait 也随之改成继承它：`ManualClockApi: abs_art::TrMockClock`——
「机制」只补 delay/interval 构造与到期表推进原语，不再重复定义能力。

### 6.2 `advance` 做成 **future**（参考 `tokio::time::advance`）

```rust
pub trait TrMockClock: TrClock {
    fn pause(&self);
    fn resume(&self);
    fn is_paused(&self) -> bool;
    fn advance(&self, by: Duration) -> impl Future<Output = ()>;
    fn advance_until(&self, at: Self::Instant) -> impl Future<Output = ()>;
}
```

理由：推进时间不是纯状态写入——被唤醒的任务需要**有机会被调度**（登记新定时器、跑完
自己的 poll）；tokio 因此把 `advance` / `advance_until` 做成 async，这里照做，调用点形状
与 tokio 一致（`rt.advance(d).await`）。

底层仍保留同步原语供驱动使用（`ManualClockApi::try_advance_to_next` /
`advance_by` / `advance_to`），因为 [`Supervisor`](crate) 在 `poll` 里不能 await。
方法名与 `TrClock` / `TrMockClock` **不重名**（`advance_by` vs `advance`，
`is_frozen` vs `is_paused`），避免两套 trait 同时 in scope 时的解析歧义。

新增用例：`async_advance_advances_when_polled`、`advance_until_never_goes_backwards`、
`body_can_advance_the_clock_itself`（主体自己 `advance(60s).await`，驱动补上剩下的）。

### 6.3 crate 名改成 `abs_art-mock_clock`（按你写的字面形式）

目录/包名从 `abs_art-mock-clock` 改为 `abs_art-mock_clock`；`lib` 名不变
（`abs_art_mock_clock`），因此代码里的路径无需改动。三后端的 feature 名仍是
`mock-clock`（feature 名与 crate 名分开）。

### 6.4 验证

`just test-mock-clock` → EXIT=0：`abs_art-mock_clock` 22 项 + 3 doctests；
tokio 23 + 8 + 1；compio 28 + 13 + 4；smol 34 + 8 + 1。三后端本 crate 0 告警。

---

## 7. 同日修正（其二）：`no_std` 为主体 + `TrMockClock` 改 GAT

### 7.1 `abs_art-mock_clock` 改成 `no_std`（`core` + `alloc`）

- 去掉对 std 的全部依赖：`Arc` → `alloc::sync::Arc`、`Vec` → `alloc::vec::Vec`、
  `Box` → `alloc::boxed::Box`、`Waker` → `core::task::Waker`、
  `std::sync::Mutex` → **同仓库的 [`atomic_sync`] 的 `SpinningMutexOwned`**（`no_std`，
  `mutex::preemptive`），而不是自己造一把锁——没必要重复发明 spinlock。
  锁的用法是 `lock_session().lock().wait()`（`atomic_sync` 的守卫借用 `LockSession`，
  因此改成闭包式 `with_(|inner| …)`，顺带保证**唤醒 waker 一定在锁外**）；
  `wait()` 的 `Err`（取消令牌）在内部用不可取消令牌时不可达，用 `loop` 把 `Result`
  收敛掉，库代码里不出现 `unwrap`/`expect`。
- `#[cfg(test)] extern crate std;`：单元测试仍用 std（测试框架需要），但**库本体**不用。
  非测试代码里 `grep "std::"` 已无命中。
- 理由（你的判断）：模拟时钟只依赖 `abs_art`；运行时是不是实质上依赖 std，不该由
  `abs_art-mock_clock` 关心。三个后端各自按需启用 `mock-clock` feature 即可。

### 7.2 `abs_art::TrMockClock` 的返回类型：RPITIT → **GAT**

```rust
pub trait TrMockClock: TrClock {
    type Advance<'a>: Future<Output = ()> where Self: 'a;
    type AdvanceUntil<'a>: Future<Output = ()> where Self: 'a;
    fn pause(&self);
    fn resume(&self);
    fn is_paused(&self) -> bool;
    fn advance(&self, by: Duration) -> Self::Advance<'_>;
    fn advance_until(&self, at: Self::Instant) -> Self::AdvanceUntil<'_>;
}
```

理由：RPITIT 的不透明返回类型会把 auto trait（`Send` / `Sync`）对调用方**藏起来**，
泛型代码里容易出现「编译器判不出这个 future 是不是 `Send`」。GAT 的返回类型是
**实现方给出的具体类型**，`Send` 与否一眼可见。两个方法各一个关联类型，因为实现方
常用两个不同的 `async` 块（本来就是两个不同的类型）。

实现侧因此改用**具名 future**：本 crate 新增 [`MockAdvance`]（`src/advance.rs`），
首次 poll 时推进（`advance` 推 `by`、`advance_until` 推绝对时刻）后立即完成。
`Advance` / `AdvanceUntil` 都指向它，因此**不需要 `impl_trait_in_assoc_type`**（ITIT）
这种 nightly 特性。

顺带一个细节：`MockAdvance` 把绝对目标装在 `Box` 里，因为 `TrClock::Instant` 只保证
`Copy + Ord + Add + Sub + 'static`，**不保证 `Unpin`**，而 `Pin::get_mut` 需要
`Self: Unpin`；`Box<T>: Unpin` 无条件成立，用一次分配换掉一层额外约束
（不愿为此给 `TrClock::Instant` 加 `Unpin`）。

### 7.3 验证

| 项 | 结果 |
| --- | --- |
| `cargo test -p abs_art-mock_clock` | 25 单测（含并发推进不丢更新的用例）+ 3 doctests |
| `just test-mock-clock` | **EXIT=0**（mock_clock 25+3；tokio 24+9+2；compio 31+16+10；smol 35+9+2） |
| `cargo test --workspace` | **24 个目标全 ok、0 失败** |
| clippy / doc | 各 crate 0 代码告警、0 断链 |

未能实测：真 bare-metal target（本机只装了 `x86_64-unknown-linux-gnu`，无法 `--target`
到 `thumbv7em-none-eabi`）。`#![no_std]` + 非测试代码无 `std::` 引用已经能保证库本体
不依赖 std。

### 7.4 追加：自旋锁改用 `atomic_sync`（不重复发明）

按你的要求，删掉了我手写的 `src/sync.rs`（`AtomicBool` + `UnsafeCell` + 两处 `unsafe`），
改为依赖同仓库的 `atomic_sync`（git source，分支 `dev/0.3.0`——与 `smux_v1` / `buffex`
的写法一致）：

```toml
# abs_art/Cargo.toml（workspace）
atomic_sync = { git = "https://gitee.com/lino_snsalias/atomic_sync.git", branch = "dev/0.3.0" }
```

```rust
use atomic_sync::mutex::preemptive::SpinningMutexOwned;

pub struct ManualClock<I: MockInstant = MillisInstant> {
    inner_: Arc<SpinningMutexOwned<Inner_<I>>>,
}
```

配套改动：

- 锁的获取变成闭包式 `with_(|inner| …)`：`atomic_sync` 的 `MutexGuard` 借用
  `LockSession`，无法从辅助函数里逃逸；闭包形式顺带把「**唤醒 waker 必须在锁外**」
  这条纪律固定在类型上（`with_` 返回后才唤醒）。
- 并发用例从「测锁」搬到「测时钟」：4 线程 × 5_000 次 `advance_by(1ms)`，断言最终恰好
  20_000ms（读-改-写若未被保护就会变小）。
- `atomic_sync` 也是 `#![no_std]`，因此 §7.1 的 no_std 结论不变；新增依赖的 git 源已在
  本机 cargo 缓存里（`cargo check --offline` 通过）。

---

## 8. `use` 语句规范整理（AGENTS.md §5）

规则原文：*「使用 `use` 语句导入依赖时，所有外部 crate（包括 core 和 std）必须以独立单根
形式出现且只出现一次」*。`cargo fmt` **不管这条**（rustfmt 的 import granularity 默认是
`Preserve`：它只排版，不合并），所以必须结构化重写。

整理后的形态（`abs_art-mock_clock` 全部 8 个文件；每个作用域里每个 crate 根只出现一次，
路径按树嵌套）：

```rust
// 之前：同一根散成多条
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use core::task::Waker;

// 之后
use alloc::{sync::Arc, vec::Vec};
use core::{fmt, future::Future, task::Waker, time::Duration};

use abs_art::{TrClock, TrInterval, TrMockClock};
use atomic_sync::mutex::preemptive::SpinningMutexOwned;

use crate::{
    advance::MockAdvance,
    delay::{MockDelay, MockInterval},
    instant::{MillisInstant, MockInstant},
};
```

做法：先用手写脚本按「crate 根 + 路径树」合并（`::` 只在顶层切分，`{...}` 组递归展开，
同根同子路径再合并），**只用 rustfmt 做排版**；随后用一个校验器逐块断言「根唯一 + 展开后
无重复路径」。留意两个坑：

1. 只按「根」合并不够——`task::Waker, task::{Context, Poll}` 属于同根下的重复子路径，
   rustfmt 也不会替你合并；
2. 朴素地按 `::` 切分会把 `{...}` 内部的 `::` 也切开（我第一版就踩了，合并等于没做）。

顺带把全 workspace 扫了一遍，另有 4 处同类违规（同一条规则）一并整理：
`abs_art-compio/src/time.rs`、`abs_art-tokio/src/local_scope.rs`、
`abs_art-smoke/src/probe.rs`、`abs_art-smoke/src/harness.rs`。现在整个 workspace 合规。

验证：`cargo test --workspace`（24 目标 ok / 0 失败）、`just test-mock-clock`（11 目标 ok）、
clippy 0 代码告警、`cargo fmt --all -- --check` 干净、`cargo doc` 0 断链。

---

## 补充（2026-10-06 13:52）：`ManualTime` 补 `Clone`

下游 `smux_v1` 在做「用 `ManualTime` 装饰运行时值」的虚拟时间验收时踩到一个缺口：
`ManualTime<R, C>` **没有** `Clone`，而消费方常要求运行时值可克隆（`smux_v1` 的
`MuxConnection::new(rt: R)` 要求 `R: Clone`——核心与循环共享量各持一份连接级时钟）。
各后端的 `Runtime` 都实现了 `Clone`，本类型作为「运行时值的装饰器」理应同样实现。

改动：`impl<R: Clone, C: ManualClockApi> Clone for ManualTime<R, C>`（克隆被装饰的值
与手动时钟各一份；`ManualClock` 的克隆共享同一份时钟状态，故两个装饰器读同一时刻、
走同一张到期表）。附带给测试替身 `support_::FakeRt_` 补上 `Clone + Copy`，
并新增单元测试 `clone_shares_the_same_manual_clock`。

性质：**公开面扩展（非破坏）**，已与人类确认。验证：`cargo test -p abs_art-mock_clock`
→ 26 passed / 0 failed。
