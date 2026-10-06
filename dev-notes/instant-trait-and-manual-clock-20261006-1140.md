# 两个问题的实测与可行性：`Instant` 要不要写成 trait；能不能用 embedded-timers 造虚拟时钟

日期：2026-10-06 11:40
分支：`feat/abs_art-runtime`
性质：**可行性实测**（本轮不改生产代码；两个问题都还没有形成决策）
探针：`abs_art_runtime_probe/p8_instant_trait/`（3 crate）、
`abs_art_runtime_probe/p6b_static_after_split/`（5 crate，含 `clock_pinning`）

---

## 1. `Instant` 要不要写成 trait

### 1.1 现状其实已经「被约束、被定义」

```rust
// abs_art/src/time.rs
pub trait TrClock {
    type Instant: Copy + Ord
        + Add<Duration, Output = Self::Instant>
        + Sub<Self::Instant, Output = Duration>
        + 'static;
    fn now(&self) -> Self::Instant;
}
```

四条约束就是「什么是时刻」的定义；具体是谁由各后端 `type Instant` 给出
（tokio → `tokio::time::Instant`；compio / smol → `std::time::Instant`）。

### 1.2 写成 trait 只有**一种**写法可行（实测）

| 写法 | 结果 |
| --- | --- |
| ① owner（`abs_art`）侧 `pub trait TrInstant: <四条约束> {}` + **blanket impl** `impl<T: <四条约束>> TrInstant for T {}` | **EXIT=0（可编）** |
| ② 非 blanket：后端给外部时刻类型手写 `impl TrInstant for tokio::time::Instant` | **`E0117`**：only traits defined in the current crate can be implemented for types defined outside of the crate |
| ② 非 blanket：给**本地**类型手写 impl（owner 已有 blanket 时） | **`E0119`**：conflicting implementations of trait `TrInstant` for type `MyInstant` |
| ③ 下游用自己的时刻类型 | 无需（也不能）手写 impl；`fn elapsed<T: TrInstant>(a: T, b: T) -> Duration` 这种短 bound 可用 |

即：**非 blanket 的 trait 写不出来**（外部时刻类型登记不进来），而 blanket 版本的能力
**严格等于**当前的结构约束——它只是一个更短的名字 + 一个能挂文档的地方。

### 1.3 与 embedded-timers 的关系（互通性论据）

`embedded-timers` 自己就同时提供了 `instant::Instant` **trait** 与 `Instant64` /
`TimespecInstant` 具体类型；而我们实测过 `Instant64<1000>` / `Instant64<32>` /
`TimespecInstant` **都满足**我们这四条结构约束。也就是说：

- **保持结构约束** → 与它天然互通，不需要实现任何 trait；
- 若另造 `TrInstant` trait，就会出现「谁的 `Instant` trait 说了算」的竞争
  （这正是 `bridge-scope-clock-20261005-1615.md` §3.3 想避开的东西）。

### 1.4 结论与建议

- 想让「时刻」有一个**公开名字与文档位置** → 可以加 blanket 版 `TrInstant`，但
  **不要**把它写进 `TrClock::Instant` 的公开 bound（保持结构约束，让名字纯属便利）；
- 想让第三方时刻类型**可插拔/可校验** → 做不到（§1.2 ②）；
- 只想要最短的公开面 → 保持现状（结构约束），把「什么是 Instant」写成 `TrClock::Instant`
  的文档（已在代码注释里）。

## 2. 虚拟时钟：能不能用 embedded-timers 给 compio / smol 补上

### 2.1 tokio 的虚拟时钟是怎么来的（源码事实）

`test-util` 打开时，runtime 内部有一个 mock clock：`Instant::now()` 读它、`sleep` 注册进它、
**runtime 空闲时自动推进**。所以 `#[tokio::test(start_paused = true)]` 下 `sleep(1h)` 瞬间完成。

实测它「钉不住」（`p6b/clock_pinning`，tokio 打开 `test-util`）：

```text
A 的值在 A 上下文读 → Instant { tv_sec: 1564761, ... }   // A 的虚拟时钟推过 1000s
B 的值在 B 上下文读 → Instant { tv_sec: 1563761, ... }
A 的值在 B 上下文读 → Instant { tv_sec: 1563761, ... }   // 读到的**是 B 的**时钟
结论：A 的值在 B 上下文读到的是 B（值钉不住时钟）的时钟
```

即 `delay` / `spawn` 钉在值上，唯独 `now()` 读**调用点上下文**的时钟（tokio 没有「读另一个
运行时时钟」的公开 API）。compio / smol 直接用进程级 std 时钟，没有这个问题。

### 2.2 embedded-timers 能提供什么、不能提供什么（源码事实）

| 提供 | 说明 |
| --- | --- |
| `clock::Clock` / `instant::Instant` trait | 时钟抽象与时刻 trait（先行者） |
| `Instant64<FREQ>` / `TimespecInstant` | **可手动推进的计数器型时刻**，天然适合当假时刻 |
| 不提供 | `timer::Timer` / `delay::Delay` 是**轮询式**（`nb::Result` / `try_wait`），**没有 waker**；也没有 `advance` / mock 时钟（`grep advance\|mock` 只命中它自己的 `mockall` 测试） |

**结论**：它能当「假时刻类型」的来源，**不能**直接充当虚拟时钟引擎。

### 2.3 可行性：可行，但要自己写「手动时钟」，且最好与后端无关

四件套：

1. **共享时刻状态**（`Cell` / `Mutex` / 原子）；
2. **按 deadline 排的 waker 注册表**（需要 `alloc`）；
3. **`TrDelay` 的 future**：注册 waker，时钟越过 deadline 时唤醒；
4. **驱动策略**（见 §2.5，唯一硬骨头）。

建议做成**装饰器值**，一份实现覆盖三个后端：

```rust
pub struct ManualTime<R> { inner: R, clock: Rc<ManualClock> }

impl<R: TrSpawnSend> TrSpawnSend for ManualTime<R> { /* 委托给 inner */ }
impl<R> TrDelay for ManualTime<R> { type Delay = ManualDelay; /* 注册到 clock */ }
impl<R> TrClock for ManualTime<R> { type Instant = Instant64<1000>; fn now(&self) -> ... }
impl<R> TrTime  for ManualTime<R> { /* Delay + Clock 同源，天然成立 */ }
impl<R> TrManualClock for ManualTime<R> { fn advance(&self, by: Duration) { ... } }
```

好处：连 tokio 也能用同一套语义 → **同一份业务代码在三后端跑同一套虚拟时间测试**；
`Instant` 直接用 `Instant64<1000>`（关联类型命名本地/第三方类型，无 orphan 问题）。

### 2.4 明确区分 SystemClock 与 ManualClock：同意，且应该用**能力 trait**

- `TrClock`（已有）=「能读这个运行时的时刻」（tokio 下可能是虚拟的，compio/smol 下是墙上时钟）；
- 新增 `TrManualClock: TrClock`（`advance(&self, by)` / `set(&self, at)`），**只有手动时钟的值实现它**；
- 于是「需要一个能手动推进的时钟」写成 `R: TrTime + TrManualClock`，而业务代码只写
  `R: TrClock` —— 两种能力在类型上分开。

### 2.5 唯一硬骨头：谁来推进时钟（三后端都有钩子，实测源码事实）

| 后端 | 空闲/推进钩子 | 自动推进循环可写性 |
| --- | --- | --- |
| tokio | `test-util` 自带：runtime 空闲时自动推进 | 不用管 |
| compio | `Runtime::run(&self) -> bool`：「Run the scheduled tasks. The return value indicates whether there are still tasks in the queue.」 | `while !done { if !rt.run() { clock.advance(step) } }` |
| smol | `async_executor::Executor::try_tick(&self) -> bool`（`LocalExecutor` 同）：文档例子 `assert!(!ex.try_tick())` = 无任务 | 同上 |

即：**时钟与 delay 本体可共用，驱动循环要各写一小段**（这是后端特有的）。不做自动推进
也能先落地：测试显式 `advance()` 再驱动。

### 2.6 落地位置

`abs_art` 是 no_std + 零依赖，**不能**放这里（waker 表需要 `alloc`；引入 `embedded-timers`
会变成公开依赖）。建议新 crate：`abs_art-manual-time`（依赖 `abs_art` + `embedded-timers`
只取 `Instant64`）。

### 2.7 建议的下一步

先做**最小探针**（而不是直接定 API）：手动时钟 + 一个后端（smol 最省事：`try_tick` +
`LocalExecutor`），验证三件事：

1. 手动 delay 能在 `run_until` 里被 `advance` 唤醒并完成；
2. 从测试体（非任务）调 `advance` 语义正确；
3. 自动推进循环（空闲检测）不死循环、不丢唤醒。

过了再谈 crate 划分、`TrManualClock` 的签名与能力位。按 `AGENTS.md` 第 1 条，
这些都是公开 API，先讨论后动手。

---

## 3. 讨论后的结论（同日追加）

### 3.1 决策：**不加** `TrInstant`

理由采纳「收益不明显、免得扩展麻烦」：具名 trait 只剩 blanket 一种形态（§1.2），
能力严格等价于结构约束，却让「任何满足约束的类型自动算」永久不可收窄。

已把这条裁决连同证据写进 [ `TrClock::Instant` 的文档 ](../abs_art/src/time.rs)
（纯文档，未动 API），未来不会再被重新提案。

### 3.2 设计建议：`TrMockClock` 是 `TrClock` 的**细化**，不是并列的第二种时钟

即 `TrMockClock: TrClock`（超 trait），**不是** `TrSysClock` / `TrMockClock` 两条并列能力。

1. **并列会重新制造「两个时钟源」的错配**——这正是 `TrClock` 存在的理由。若一个 mock 值
   同时实现「真实时钟」与「虚拟时钟」，`TrTime: TrDelay + TrClock` 只能绑定其中一个
   `Instant`：业务代码读到真实时刻、而 `delay` 睡在虚拟时钟上 → smux 那个「空闲超时永不
   触发」的 bug 原样复现（见 `bridge-scope-clock-20261005-1615.md` §3.1）。而并列的写法
   **逼着** mock 值同时实现两个时钟，否则业务代码在 mock 下根本不可用。
2. **Rust 表达不了 `R: TrSysClock + !TrMockClock`**，所以「并列」买不到任何强制力，
   却付出了两个源的代价。细化模型反而给到需要的东西：一个值只有**一个**时钟，
   `TrTime` 的同源性质不变；测试要求 `R: TrMockClock`、业务只要求 `R: TrClock`，
   同一份业务代码在真实与虚拟时间下**零改动**跑（tokio 也是这个模型）。
3. **tokio 原生符合细化模型**：`test-util` 下 `Instant::now()` 与 `pause()` / `advance()`
   用的是同一个 `tokio::time::Instant`、同一个时间源 → tokio 后端可以**原生**实现
   `TrMockClock`；compio / smol 由 `ManualTime<R>` 装饰器实现同一个 trait。
   一个 trait 覆盖「原生 + 装饰」两种来源。
4. **命名**：建议保留基名 `TrClock`，新增 `TrMockClock: TrClock`。不建议把基名改成
   `TrSysClock`：mock 值的 `TrClock::Instant` 是虚拟时刻，叫 `Sys` 名不副实。
   若日后确实需要「虚拟时钟下读真实墙上时间」（例如给日志打真实耗时），那是**第三种**
   关切，应单列 `TrWallClock`（值可委托给 std），而不是占用基名——目前 YAGNI。

**能力位**：一条 `CLOCK` 就够（与 §待裁决的那条合并）。`TrMockClock: TrClock` 决定了
「有 mock 能力必然有 clock 能力」，不需要第二位；而「声明」的落点是**构造**——mock 值只能
由测试显式构造（`ManualTime::new(inner)`，或显式调用 tokio 的 `pause()`），不存在
「不小心选到 mock」的静默风险。

### 3.3 待探针确认的签名问题

tokio 的 `advance(d)` 是 **async** 的（要把推进交给运行时），装饰器的可以是同步的。
统一形状的草案：

```rust
pub trait TrMockClock: TrClock {
    fn pause(&self);                        // tokio: time::pause()；手动时钟: 冻结
    fn resume(&self);
    fn advance(&self, by: Duration) -> impl Future<Output = ()>;  // tokio 侧需要 await
}
```

另需确认：(a) tokio 的 `pause()` 仅支持 `current_thread` 运行时（p6b 已留下报错原文），
这条前提要不要写进 trait 文档；(b) 时钟「钉不住值」的性质（§2.1）在 mock 上是分裂的
——装饰器的 `advance` 钉在值上，tokio 原生的 `advance` 是环境式的。
