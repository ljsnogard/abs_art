# 三项构想：bridge 取消「只能一个后端」、thread-local 的 `LocalScope`、`Clock` 作为独立 cap

日期：2026-10-05 16:15
性质：**构想 + 证据收集**（**本轮不动代码**）。三节各给现状、可复现证据、构想、待裁决。
来源：`smux_v1` 落保活（`dev-notes/keepalive-timer-loop-20261005-1420.md`）时暴露的三处
「能力放在哪里」的问题。
前置阅读（**注意**：其中前两份在本轮开始时已从工作区删除，`git status` 为 `D`、HEAD 中仍有
——本文把其中的关键结论原文引出，以免依赖一个可能被清理的文件）：
`spawn_local-20261002-1247.md`（路线 1/2/3 的取舍）、`local_scope-20261002-1307.md`
（路线 1 的落地）、`time-20261005-1225.md`（`TrTime` 的落地）。

---

## 0. 三项是同一条线

它们都在问同一个问题：**一项能力该由「值」承载、由「类型」承载、还是由「环境」承载？**

| 项 | 今天挂在 | 构想挂到 |
| --- | --- | --- |
| §1 bridge 的后端选择 | Cargo feature + 一条 `compile_error!` 硬限制 | 编译条件（多后端时按名导出） |
| §2 本地队列 | `LocalScope` **值**（调用方持有、驱动、克隆） | 后端持有的 thread-local **环境** |
| §3 「现在几点」 | `smux` 侧一个配置项（`TrConnCfg::Clock`） | `abs_art` 的**独立 capability**（类型级） |

三者的先后关系：§3 独立、两条路都要；§1 独立、小；§2 最大，且会决定 §3 的最终归属
（若队列进环境，能力表也会跟着进环境）。

**证据等级约定**：本文凡涉及运行时行为的结论，标 `[实测]`（附命令与原始结果）或
`[源码事实]`（附文件:行）或 `[推断·未实测]`（附该做哪个实验）。

---

## 1. `abs_art-bridge` 取消「只能集成一个后端」的硬限制

### 1.1 现状

`abs_art-bridge/src/lib.rs` 做两件事：

1. 按 `backend-*` feature 把某个后端的 `Runtime` / `LocalScope` **同名**重导出
   （`:55` / `:59` / `:63`），使业务代码只写 `Runtime`、切换后端只改 Cargo.toml；
2. 两条硬限制（`:86` 必须启用一个、`:98` **只能启用一个**）。

### 1.2 那条硬限制不是语言约束，它拦的是**重名**

`[实测]` 把「两个同名重导出」写进一个文件：

```rust
mod tokio_be   { pub struct Runtime; }
mod compio_be  { pub struct Runtime; }
pub use tokio_be::Runtime;
pub use compio_be::Runtime;
```

```text
error[E0252]: the name `Runtime` is defined multiple times
 --> c.rs:4:9
  |
3 | pub use tokio_be::Runtime;
  |         ----------------- previous import of the type `Runtime` here
4 | pub use compio_be::Runtime;
  |         ^^^^^^^^^^^^^^^^^^ `Runtime` reimported here
  = note: `Runtime` must be defined only once in the type namespace of this module
help: you can use `as` to change the binding name of the import
```

`[实测]` 同一条件（两个后端同时启用）改成按名导出即可编译通过：

```rust
pub use compio_be::Runtime as CompioRuntime;
pub use tokio_be::Runtime as TokioRuntime;
```
```text
OK: 按名导出双后端编译通过
```

**结论**：两个后端同时链接**没有任何语言层面的问题**——它们是两个不同的类型，各有一份
`impl`，互不冲突。今天的硬限制真实来源是「同名重导出 + 一条策略性 `compile_error!`」。
`compile_error!`（`:98`）实际上是在替 E0252 说一句人话。

### 1.3 这条硬限制已经造成了绕过（家族内部证据）

`[实测]`/`[源码事实]` `abs_art-smoke/Cargo.toml:13-16` 的原文：

> 本 crate 只做一件事：把**同一份**测试体分别放到 tokio / compio / smol 三个真实运行时
> 上跑……因此三个后端 crate 同时是依赖，且**不经过** `abs_art-bridge`——bridge 的
> `backend-*` feature 互斥，一次构建只能启用一个，装不下「三个后端同时对比」这个需求。

`[实测]` 三后端矩阵当前是绿的（本轮重跑）：

```console
$ cargo test -p abs_art-smoke
     Running tests/spawn_local_contract.rs
running 12 tests
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
     Running tests/time_contract.rs
running 18 tests
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

即：**三个后端同处一个二进制、同一进程被横向对比**是既成事实，只是必须绕开 bridge。

`[源码事实]` 代价还记在 `abs_art-bridge/Cargo.toml` 的 features 注释里：`cargo test
--workspace` 会对同一个 `abs_art-bridge` 的 feature **取并集**，一旦并集里出现两个
backend 就触发 `compile_error!`，于是「缺省 backend 必须与 workspace 里其它成员所用的
后端一致，改一处必须同时改另一处」。

### 1.4 为什么不能反过来：把唯一的 `Runtime<CAPS>` 定义进 `abs_art`

`[实测]` 孤儿规则直接否掉（crate `a` 定义 trait 与类型，crate `b` 试图实现）：

```rust
// a.rs
pub trait TrDelay { fn delay(); }
pub struct Runtime<const CAPS: usize>;
// b.rs
impl a::TrDelay for a::Runtime<1> { fn delay() {} }
```
```text
error[E0117]: only traits defined in the current crate can be implemented for types
              defined outside of the crate
 --> b.rs:1:1
  |
1 | impl a::TrDelay for a::Runtime<1> { fn delay() {} }
  | ^^^^^^^^^^^^^^^^^^^^-------------
  |                     `Runtime` is not defined in the current crate
  = note: impl doesn't have any local type before any uncovered type parameters
```

这解释了**为什么 `Runtime<CAPS>` 今天必须由每个后端 crate 自己定义**：只有类型是后端
本地的，`impl TrDelay for Runtime<CAPS>` 才合法。

可行的替代是 `abs_art::Runtime<B, CAPS>`（`B` 是后端本地的标记类型，后端
`impl Backend for B` 合法——trait 外来、类型本地），但调用点仍要写 `B`，等于把「每后端
一个类型」换个写法、并没有省掉名字。

### 1.5 构想

1. **多后端时按名导出**：`abs_art_bridge::TokioRuntime` / `CompioRuntime` / `SmolRuntime`
   （或子模块 `abs_art_bridge::tokio::Runtime`），**同一编译条件下可并存**；
2. **单后端时保留裸名 `Runtime` / `LocalScope`**（现有用法零改动）；
3. `compile_error!` 从「只能一个」降级为「同名冲突时给出 `as` 提示」或直接取消；
   `abs_art-bridge/Cargo.toml` 里那段 feature 并集注释随之删除；
4. `abs_art-smoke` 可以改成**经 bridge** 取三个后端的名字（不再绕过），
   从而顺带验证 bridge 在「三后端同存」下确实可用。

### 1.6 待裁决

- 单后端时是保留裸名，还是统一成按名 + 再给一条 `cfg` 裸名别名？
- 三后端时命名规则（前缀 vs 子模块）；是否顺带给 `LocalScope` 同样的按名版本。
- 这条改动与 §2 有交集：若 §2 让 `LocalScope` 退化成句柄/零状态，按名导出的内容也要跟着变。

---

## 2. 基于 thread-local 的 `LocalScope`

### 2.1 现状：三个后端的「谁持有队列」并不一致

`[源码事实]`（三个后端的 `local_scope.rs`）：

| 后端 | `LocalScope` 里装什么 | 队列归谁 | 谁驱动 |
| --- | --- | --- | --- |
| compio | **`pub struct LocalScope;`（ZST）** | 运行时（线程本地队列） | 运行时 |
| tokio | `Rc<LocalSet>`（每个值新建一条） | 值（调用方） | 值的 `run_until` / `block_on` |
| smol | `Rc<LocalExecutor<'static>>`（每个值新建一条） | 值（调用方） | 值的 `run_until`（`LocalExecutor::run`） |

两点直接推论：

1. **compio 已经就是「环境持有队列」的形状**——它的 scope 是空的，`smux` 在
   compio 上「持有作用域值」其实是持有一个毫无意义的 ZST；
2. 另外两个后端把队列放进 thread-local 在技术上完全可行（三者都在 std 环境下，
   `thread_local!` 可用）。

> 顺带更正一处旧记录的细节：`spawn_local-20261002-1247.md` 的路线 1 对照表把 smol 写成
> 「每线程一个 `Rc<LocalExecutor>`」，但**实现不是**——每个 `LocalScope::new()` 新建一条。
> 因此「线程本地缓存执行器」在家族里既无先例也无反例。

### 2.2 与早前「路线 2」的关系（必须写清，否则会被当成走回头路）

早前的调研把「thread-local 隐式本地队列」列为**路线 2 并否决**，原文的理由是：

> **致命处：「谁驱动」仍然没有答案。** tokio 的 `enter()` 只是把上下文设好（使自由函数
> 不 panic），**它不会驱动队列**；smol 那边同理。因此仍然必须有一个显式的驱动入口……
> 问题只是从「环境前提」挪到了「驱动入口」，并没有消失。
> 附带代价：隐式全局状态带来嵌套 / 多实例 / 跨运行时语义的复杂度。

**本构想与路线 2 的差别**：路线 2 是「thread-local + **自由函数** `spawn_local` + 仍要
调用方自己找驱动入口」；本构想把**驱动入口也交给后端**——`run_until` / `block_on` 作为
后端类型的关联函数，自己从 thread-local 取队列并驱动。于是「谁驱动」有答案了：**后端**。
原否决意见针对的形态与它不同。

### 2.3 未解的硬点：能力表放在哪

`spawn_local` 一旦变成关联函数，**库内部就必须命名那个类型**：

- `<S as TrLocalScope>::spawn_local(..)` → 需要 `S`（回到泛型参数）；
- `abs_art_tokio::LocalScope::spawn_local(..)` → 库要依赖 tokio。

要真的「谁都不用命名」，只能让 `abs_art` 自己持有一份**按线程安装的能力表**，后端在驱动
入口把它装上。而 `abs_art` 是 `no_std` + 零依赖，`thread_local!` 需要 std。两个候选：

| 候选 | 做法 | 代价 |
| --- | --- | --- |
| (a) `abs_art` 开 `std` feature | 能力表在 `abs_art` 内部 | `abs_art` 的 no_std 边界要按 feature 切开；无 std 的消费者拿不到环境入口 |
| (b) 新增/改造一个 **std 桥 crate** | 桥持有 TLS 能力表 + 驱动入口；`abs_art` 只声明 trait | 多一个 crate；库（如 `smux`）要依赖这个 std 桥 |

**这是本项必须先裁决的一件事。** 倾向 (b)：`abs_art` 的 no_std / 零依赖边界不动。

### 2.4 代价清单（不是净收益，必须写下来）

1. **每线程一条队列**：今天可以造多条、可以嵌套；thread-local 之后一个线程一条。
   测试隔离也随之改变——今天每个用例 `LocalScope::new()` 拿到全新队列，之后同一线程上的
   用例共用一条，前一个用例泄漏的任务会漏到下一个（今天由构造保证不会）。
2. **寿命从「值」变成「线程」**：应用不再能提前回收队列（这一条反而比今天好——今天要在
   文档里警告「别先丢 scope」），但也失去「作用域结束即收尾」。
3. **`Send` 边界要改断言**：今天 tokio 侧连接是 `!Send`（`Rc<LocalSet>` 在 `MuxCore` 里）、
   compio 侧是 `Send`，`smux_v1/tests/thread_safety.rs` 把这条边界钉成公开行为；去掉值
   之后 tokio 侧也会变 `Send`。
   **自我更正**：本轮早前说过「移动连接会把循环投到别的线程的队列上」，那句话说过了——
   `spawn_local` **只在 `MuxConnection::new` 里发生**，连接移动之后不会再 spawn。这条只是
   「被钉住的公开行为要改」，不是安全性问题。

### 2.5 收益

`MuxConnection<C>`：`smux` 侧 **109 处 `<C, S>`** 塌缩、7 个公开类型变简单、`S: Clone`
不再需要、`TrLocalScope::Runtime` 投影不再需要（见 §3.4）、`TrTime` / `TrClock` 自然落回
运行时类型。**这是目前唯一能把「运行时参数」从公开面上彻底拿掉的方案。**

### 2.6 需要什么实验 / 裁决

- `[推断·未实测]`「后端持有的 thread-local 队列 + 后端驱动入口」能替代值化。机制上成立
  （compio 已是先例），但 tokio / smol 的版本**未实现也未实测**。实验：不动公开 API，
  先把 `LocalSet` / `LocalExecutor` 放进各后端的 `thread_local!`，跑
  `cargo test -p abs_art-smoke`（两个契约矩阵）+ `smux_v1` 的全套（`just test`）对比。
- 裁决 (a)/(b) 能力表归属；裁决「同一线程先后进两个后端」的语义（安装/卸载、嵌套）。

---

## 3. `Clock` 作为独立 capability

### 3.1 动机

`smux` 把「等待」与「现在几点」拆成两个旋钮：等待来自后端（`S: TrTime`），时刻来自配置
（`TrConnCfg::Clock`）。**没有任何东西保证两者同一个时间基准。**

`[推断·未实测]` 具体踩法：`SystemClock` 读 `std::time::Instant`（墙上时钟），而
tokio 的 `sleep` 睡在**可暂停的** `tokio::time::Instant` 上。于是

```rust
#[tokio::test(start_paused = true)]        // 虚拟时间
type Clock = smux_v1::time::SystemClock;   // 读墙上时钟
```

虚拟时间被 `sleep` 自动推进，而 `smux` 用墙上时钟算出的 idle 几乎不动——**空闲超时永远
不触发**，测试「不挂」但断言的是另一回事。

验证实验：给 `smux_v1/tests/keepalive_common.inc` 加一个 `start_paused` 变体
（tokio dev-dep 需加 `test-util`），断言虚拟时间推进后超时触发；再把时钟换成
`tokio::time::Instant` 作对照。**本项在本文之前没有实测数据。**

### 3.2 现状事实

`[源码事实]` `TrDelay::delay` / `TrTime::interval` 都是**关联函数**
（`abs_art/src/runtime.rs:268-277`），即：**能力只需要一个类型名，不需要任何值**。

`[源码事实]` 「现在几点」在三个后端的出处：

| 后端 | 时刻类型 | 出处 |
| --- | --- | --- |
| tokio | `tokio::time::Instant`（可 `pause()` / `advance()`，`test-util`） | `tokio-1.53.1/src/time/instant.rs:34` |
| compio | `std::time::Instant` | `compio-runtime-0.12.6/src/time/mod.rs:7`（`time::{Duration, Instant}`，整段 `time` 模块的 `sleep_until` 收的就是它） |
| smol | `std::time::Instant` | `async-io-2.6.0/src/lib.rs:70`（`use std::time::{Duration, Instant}`）+ `:234`（`Timer::at(instant: Instant)`） |

即：**「计时器」是三个运行时的共同能力，「时钟」只有 tokio 有自己的**（理由是
`test-util` 的虚拟时间与它自己的时间驱动记账），另外两个直接就是 std。

`[源码事实]` 这解释了 `abs_art` 为什么在 `time-20261005-1225.md` 里**刻意不暴露
`Instant`**：它是 no_std + 零依赖，既不能写 `std::time::Instant`，也不该为此引入一个外部
时钟抽象 crate（那会成为 `abs_art` 的公开依赖）。

### 3.3 构想：`TrClock`，独立于 `TrDelay`

```rust
// abs_art（零依赖、no_std 不变）
pub trait TrClock {
    /// 用**结构约束**表达时刻类型，而不是引入/复制一条 `Instant` trait。
    type Instant: Copy + Ord
        + Add<Duration, Output = Self::Instant>
        + Sub<Self::Instant, Output = Duration>
        + 'static;
    fn now() -> Self::Instant;
}
```

三点设计意图：

1. **不继承 `TrDelay`**（与 `TrTime: TrDelay` 相反）：`TrClock` 不为异步服务，它只是
   「获得时刻」；需要两者的消费者自己写 `+ TrTime + TrClock`。
2. **结构约束而不是自己的 `Instant` trait**：`std::time::Instant`、`tokio::time::Instant`
   （`instant.rs:33` derive `Copy/Ord`、`:165` `Add<Duration>`、`:179` `Sub<Instant>`）、
   `embedded_timers::instant::Instant64` 这类 tick 计数器（这些正是它那条 `Instant` trait
   的 supertrait）**都满足**。于是 `abs_art` 既不依赖 `embedded-timers`，也不与它争
   「谁的 `Instant` trait 说了算」。
3. **能力位**：可以复用 `DELAY`（`caps.rs` 里它的语义已经是「本后端具备计时能力」），
   也可以新开一位 `CLOCK`——后者的收益是 `caps.rs` 那条「想用就得写下来、可 grep」，
   而且「时刻」确实不依赖时间驱动，语义上独立。

### 3.4 与 `LocalScope` 的关系

`TrTime` 现在实现**在 `LocalScope` 上**（三个后端各一份），理由记在
`time-20261005-1225.md`：业务库手上只有作用域值，补上这两格就能只加一条约束、不引入
第二个类型参数。但按 §0 的分法，`TrDelay` / `TrClock` 与调度无关（本地/全局调度共用同一类
Delay），挂在「本地队列的值」上是不合适的。

两种收尾：

- **若走 §2（环境化）**：能力自然从 `LocalScope` 移到「后端类型 + 线程本地能力表」，
  `LocalScope: TrDelay/TrTime` 这两份实现可以删掉（它们当年只为 `smux` 的 `S: TrTime` 而存在）；
- **若暂不走 §2**：至少把「作用域**说出**自己属于哪个运行时」表达出来
  （`TrLocalScope::Runtime` 关联类型），`smux` 侧把 `S: TrTime` 换成
  `S::Runtime: TrTime + TrClock`——值仍然必须被取得与驱动（路线 1 的理由一条不丢），
  但 Delay/Clock 回到运行时类型上。

### 3.5 未决

**时钟是「环境能力」（关联函数、无值）还是「可注入的值」？**

- 只留环境能力：少一个旋钮，且**结构上杜绝**时钟与计时器不同源；确定性验收改成「换一个
  假的后端类型」。
- 环境能力作缺省 + 保留注入覆盖：`smux` 今天依赖注入（`TrConnCfg::Clock`）做确定性验收，
  且「真实计时器 + 自备时钟（观测/指标）」是合理用法；代价是要多一层「缺省 vs 覆盖」的包装。

无论选哪个，**缺省必须与后端计时同源**——这是本节唯一不可让步的部分。

---

## 4. 建议顺序与影响面

| 顺序 | 项 | 独立性 | 会返工吗 |
| --- | --- | --- | --- |
| 1 | §1 bridge 放宽 | 完全独立、小 | 不会 |
| 2 | §3 `TrClock` | 独立（两条路都要它） | 不会 |
| 3 | §2 thread-local `LocalScope` | 最大，需先裁 (a)/(b) | 会决定 §3.4 的最终归属 |

非目标（本文）：不改任何代码；不恢复工作区里那两个 `D` 状态的旧文档。

---

## 5. 证据索引（可复现）

| 编号 | 结论 | 命令 / 来源 | 结果 |
| --- | --- | --- | --- |
| E1 | 三后端同处一个二进制、同进程对比是可用的 | `cargo test -p abs_art-smoke` | `spawn_local_contract` 12 passed、`time_contract` 18 passed，`EXIT=0` |
| E2 | bridge 的硬限制拦的是**重名** | `rustc --crate-type=lib` 编译两个同名 `pub use` | `error[E0252]: the name 'Runtime' is defined multiple times` |
| E3 | 同一条件改成按名导出即可通过 | 同上，改成 `as TokioRuntime` / `as CompioRuntime` | 编译通过 |
| E4 | 唯一 `Runtime<CAPS>` 定义在 `abs_art` 不可行 | `rustc`：crate a 定义 trait+类型、crate b 实现 | `error[E0117]`（孤儿规则） |
| E5 | 硬限制已造成家族内部绕过 | `abs_art-smoke/Cargo.toml:13-16` 原文 | 「不经过 `abs_art-bridge`……装不下三后端同时对比」 |
| E6 | feature 并集的代价 | `abs_art-bridge/Cargo.toml` features 注释 | 「改缺省值必须同时改 demo 的缺省演示组」 |
| E7 | 三个 `LocalScope` 的持有物不同 | 三个后端 `src/local_scope.rs` | compio ZST；tokio `Rc<LocalSet>`；smol `Rc<LocalExecutor<'static>>` |
| E8 | 能力只需要类型名 | `abs_art/src/runtime.rs:268-277` | `fn delay(duration) -> Self::Delay`（无 `self`） |
| E9 | 「时钟」只有 tokio 有 | tokio `time/instant.rs`；compio `time/mod.rs:6-9`；async-io `lib.rs:70` | tokio 自有 `Instant`；另两者用 `std::time::Instant` |

`[推断·未实测]` 的两条（§2.6、§3.1）各自附了该做的实验；本节不把它们列成证据。
