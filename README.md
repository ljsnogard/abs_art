# abs_art

ABStraction of Asynchronous RunTime —— 一次编写，任意异步运行时。

## 为什么要用 abs_art？

写一个会被别人集成的库时，你迟早要面对这个选择：

```rust
// 你的业务库：想 spawn 一个任务，但……
tokio::task::spawn(async { ... });          // 选 tokio → 用户只能用 tokio
compio::runtime::spawn(async { ... });      // 选 compio → 用户被绑死在 compio
smol::spawn(async { ... });                 // 选 smol → 用户想换都不行
```

更糟的是，就算你改成泛型：

```rust
pub async fn process<R>(...) -> ...          // 每个函数都要带一个 R 参数
where R: TrSpawnSend + TrDelay + ...         // 泛型参数一路穿透所有代码
```

> **本版（0.3.0，开发中）的几次收敛。** 能力 trait 不再带 `F` 类型参数：被 spawn 的
> future 类型是各自动**方法级泛型参数**，于是一个 `R: TrSpawnSend` 约束就能覆盖该运行时上
> **所有**任务类型（包括调用点无法命名的 `async {}` 块）。**本地投递**则经过四轮收敛，
> 最终定型为「声明位 + 线程本地队列的别名」：
>
> 1. 最初是类型级 `TrSpawnLocal<F>`；
> 2. 后来拆成「能力位 `SPAWN_LOCAL`（只负责声明）+ `TrLocalScope` 值」，但那个作用域
>    可以脱离运行时凭空造出来；
> 3. 中途一度把本地队列并进**运行时值**（`rt.spawn_local(..)`），想让「哪个运行时」
>    由手上的值回答；代价是 tokio 的运行时值被迫 `!Send`——`Rc<LocalSet>` 拖累了本来
>    `Send + Sync` 的 `Handle`，而「可共享的把手」与「线程独占的队列」两种生命周期
>    被绑死；
> 4. **最终定型**：`Runtime` 是**值**（`&self` 能力方法：全局投递 / 阻塞等待 / 计时与
>    时刻）；`LocalScope` 是**本线程那条队列的别名**——tokio / smol 把队列放进
>    `thread_local!`，compio 的队列本就在线程本地的运行时实例里（它是本次对齐的基准）。
>    三条可依赖的性质：`Clone` 只增加别名、同一线程上多次 `local_scope()` 拿到**同一条**
>    队列、类型是 `!Send`；「同一线程多条 `LocalSet`」被显式排除；
>    新增 `TrClock`（`TrTime: TrDelay + TrClock`）让「时刻与计时器同源」成为类型约束；
>    **compio 不实现 `TrSpawnSend`**——它没有跨线程全局队列，假装有会让约束失真。
> 5. **作用域的阻塞入口回来了，但换了机制**：`TrLocalScope::block_on_local` 提供
>    「阻塞当前线程直到 future 完成，等待期间持续驱动本线程队列」。它不是当初那条被删的
>    `scope.block_on`（那条把「怎么阻塞」整个交给后端，tokio 上只能用 `block_in_place`，
>    在 `LocalSet` 内被直接禁止）；新方法**不使用任何运行时的阻塞原语**：tokio / smol 用
>    「`run_until` 驱动队列 + 纯 park」，compio 自己 tick 执行器与 IO。三端接口与可观察
>    语义一致，差异只剩「本线程的 IO 由谁推」，如实写在文档里。
>    收敛过程与实测见 `dev-notes/`。

你的 API 被"运行时类型"污染，业务逻辑里全是与业务无关的泛型噪音。

**abs_art 的思路：把"选哪个运行时"从"写代码的时候"推迟到"集成的时候"。**

- **业务库**只声明一句话：*我需要 `block_on` + `spawn_send` 这两种能力*，完全不感知 tokio / compio / smol；
- **集成方**（通常是最终的二进制）在 `Cargo.toml` 里用一行 feature 决定后端；
- 切换后端 = 改一行配置，**业务库代码零改动**；
- 想要的编译期保证一个不少：**调用了没有声明（或后端不支持）的能力，直接编译报错**。

## 快速上手：真实业务的写法

### 业务库侧（不感知任何后端）

```toml
# 业务库的 Cargo.toml：只依赖 bridge，不选后端（default-features = false）
[dependencies]
abs_art-bridge = { path = "abs_art-bridge", default-features = false }
```

```rust
// 业务库 lib.rs —— 只约束能力，不关心后端，也不关心容器类型
use abs_art_bridge::{TrBlockOn, TrDelay, TrLocalScope};

/// 业务函数 A：只约束**三个后端共同**的能力（计时 + 阻塞等待）
pub fn double_after_tick<R>(rt: &R, x: u32) -> u32
where
    R: TrBlockOn + TrDelay,
{
    rt.block_on(async move {
        rt.delay(core::time::Duration::from_millis(1)).await;
        x * 2
    })
}

/// 业务函数 B：本地投递（`!Send` 任务）——需要一个**线程独占**的作用域值
pub async fn local_rc_double<S>(scope: &S, x: u32) -> u32
where
    S: TrLocalScope,
{
    let rc = std::rc::Rc::new(x); // !Send：只有本地队列能承载
    scope.spawn_local(async move { *rc * 2 }).await.unwrap()
}
```

本版定型的形状：**运行时是值**，**本地队列是本线程那条队列的别名**：

| 关切 | 从哪调 | 为什么 |
| --- | --- | --- |
| `spawn`（全局队列） | **运行时值** `rt.spawn(..)` | 队列可跨线程共享，值是它的把手（tokio / smol） |
| `block_on` / `delay` / `interval` / `now` | **运行时值** | 与「在哪个线程调度」无关 |
| `spawn_local` / `run_until` / `block_on_local` | **作用域值** `scope.…` | 队列绑定线程，而作用域只是本线程那条队列的别名 |
| 同步等待 **且** 驱动队列 | **作用域值** | `scope.block_on_local(f)`——由后端自己驱动队列，不借运行时的阻塞原语 |
| 只等待、**不**驱动队列 | **运行时值** | `rt.block_on(f)`（tokio 上是 `block_in_place`，本地驱动栈内不可用） |

作用域从运行时值交出：`rt.local_scope()`，**要求 `CAPS` 含 `SPAWN_LOCAL`**——
想用本地投递，就得把这件事写在类型上；同一线程上多次取得拿到的是**同一条**队列。

**compio 不实现 `TrSpawnSend`**：它没有跨线程全局队列，`spawn` 投的是本线程运行时的
队列，因此「三后端共用」的业务代码只能建立在共同子集上（上例的业务函数 A）。

### 集成方侧（通过 Cargo.toml 选后端）

```toml
# 二进制的 Cargo.toml：一行 feature 决定后端；再加直接依赖用来构造运行时
[dependencies]
abs_art-bridge = { path = "abs_art-bridge", default-features = false, features = ["backend-tokio"] }
tokio = { version = "1", features = ["rt", "rt-multi-thread"] }
```

```rust
// main.rs —— 唯一感知后端的地方
fn main() {
    let outer = tokio::runtime::Runtime::new().unwrap();
    let out = outer.block_on(async {
        // 构造运行时**值**：全能力（也可写 Runtime::<{ .. }>::current() 只给子集）
        let rt = abs_art_bridge::current();
        let scope = rt.local_scope();          // 线程独占的本地作用域
        let a = my_business_lib::double_after_tick(&rt, 21);
        let b = my_business_lib::local_rc_double(&scope, 21).await;
        (a, b)
    });
    assert_eq!(out, (42, 42));
}
```

想换 compio / smol？改 `backend-tokio` → `backend-compio` / `backend-smol`，并按需调整 `main.rs` 里创建运行时的代码。**业务库一行都不用动。**

## 依赖结构与原理

### Workspace 结构

```
abs_art            基础 crate：不依赖任何运行时
                    ├─ enum RuntimeTag（运行时身份）
                    ├─ trait：TrAsyncRuntime / TrBlockOn / TrSpawnSend /
                    │          TrSpawnBlocking / TrDelay / TrClock / TrTime /
                    │          TrJoinHandle
                    │          ※ 全部是 **&self 方法**：能力由运行时「值」提供
                    │          （TrJoinHandle::detach：smol/compio 原生支持，
                    │           tokio 无原生 detach，drop 句柄即 detach——语义等价）
                    ├─ trait：TrLocalScope —— **本线程队列的别名**
                    │          （spawn_local / run_until / block_on_local；
                    │           计时与时刻不在这里）
                    └─ caps：能力位掩码（BLOCK_ON / DELAY / SPAWN_SEND /
                              SPAWN_LOCAL / SPAWN_BLOCKING / CLOCK）与类型级标记
                              ※ 位负责「声明」，且真正门控调用点；每个后端另有
                                自己的 `FULL`（compio 的 FULL 不含 SPAWN_SEND）

abs_art-tokio      tokio 后端：Runtime<const CAPS> = Handle（Send + Sync）；
                   LocalScope = 本线程 thread_local! 里那条 LocalSet 的别名
                   （Rc<LocalSet> + 一个 Handle，!Send）
abs_art-compio     compio 后端：Runtime 持 compio 运行时（本身线程绑定）；
                   LocalScope 持同一份运行时（形状对齐的**基准**）；
                   **声明 SPAWN_SEND 即静态失败**（自己的 FULL 不含该位）
abs_art-smol       smol 后端：Runtime 是零大小标记（全局执行器进程级）；
                   LocalScope = 本线程 thread_local! 里那条 LocalExecutor 的别名

abs_art-bridge     桥接：backend-tokio / backend-compio / backend-smol 可多选，
                   裸名 Runtime 默认哪个由 cfg 优先级 / 显式 default-backend-* 决定；
                   另给具名别名 TokioRuntime / CompioRuntime / SmolRuntime

abs_art-demo       演示：业务库（零泛型穿透）+ 二进制（选后端）
                    examples/ 下按后端分组（tokio_demo / compio_demo）的
                    每种 cap 一个 smoke test（features：demo-compio 默认 / demo-tokio）

abs_art-smoke      跨后端 spawn_local 行为契约冒烟测试（publish = false）
                    同一份测试体（本地投递泛型于作用域值 S: TrLocalScope，时间泛型于运行时值 R: TrTime）分别跑在三个
                    真实运行时上，输出 3 后端 × N 用例的对比矩阵（全绿）。
                    测试体不走 bridge——它要三个后端同时对比
```

依赖关系（业务库只碰 bridge）：

```
业务库 ──► abs_art-bridge ──► abs_art-tokio / abs_art-compio / abs_art-smol（可多选）
                     └──────► abs_art（基础）
```

### 为什么是零开销

1. **能力检查发生在编译期**：`CAPS` 是 const 位掩码，`[(); CAPS]: HasBlockOn` 这类类型级标记在编译期被求解，决定这个运行时值实现了哪些能力 trait。
2. **调用是静态分发**：`rt.block_on(...)` / `rt.spawn(...)` 在编译期被单态化为直接调用 tokio/compio/smol 的 API。**没有 vtable、没有 `Box`、没有 downcast、没有动态分发**——最终机器码与手写后端调用等价。
3. **运行时值本身很小**：tokio 版只有一个 `Handle`（本地队列在 `thread_local!` 里），没有任何按能力构造的数据结构；`spawn` / `delay` / `now` 的路径上只有一次直接调用。
3. **后端 crate 的五个功能（`block_on` / `delay` / `spawn_send` / `local_scope` / `spawn_blocking`）是 feature 开关**：按需编译，不用的代码不进产物。

对比其它方案：运行时注入（log 风格）需要类型擦除 + 装箱 + downcast，每次调用都有开销；泛型穿透需要把 `R` 参数写进每个函数签名。abs_art 用"能力声明"把两者都省掉了——零开销，且签名干净。

### 本地投递：一个声明位 + 一个作用域值

`spawn_local` 仍然是两半，**职责不同、不互相替代**——但本版「值」的那一半换人了：

| | 回答的问题 | 载体 |
|---|---|---|
| 能力位 `SPAWN_LOCAL` | 你**声明**了没有？ | `Runtime<CAPS>` 的类型级标记 |
| 作用域值 | 你**拿到**了没有？ | 实现 `TrLocalScope` 的 `LocalScope`（本线程队列的别名） |

#### 位负责「声明」

位**拦不住**真想用的人——把位写上就够了。它的价值在于**强制显式**：想开始用本地投递，就必须先把这件事写下来，于是它必然出现在类型别名、diff 与 code review 里，也能被 `grep` 出来。最贴切的类比是 `unsafe`：任何人都会写，但**必须写**。

```rust
use abs_art_bridge::{Runtime, SPAWN_LOCAL, TrLocalScope};

// 声明：这次「升级」留了痕；位门控「取得作用域」这唯一入口
let rt = Runtime::<{ SPAWN_LOCAL }>::current();
let scope = rt.local_scope();                  // ← 只有写了位才拿得到
let handle = scope.spawn_local(async { 1u32 }); // ← 投递点在作用域上
```

作用域的构造函数**不公开**：只能从运行时值取得，因此不会出现「作用域与运行时无关」
的串（这正是原设计需要写免责声明的原因）。

（仍然不是安全边界：任何人都可以写 `Runtime::<{ SPAWN_LOCAL }>`。位的价值是
「你必须写下来」，约束的是**意外**，不是**恶意**。）

#### 作用域负责「事实」

`spawn_local` 除了「运行时支持」之外还有一条**环境前提**：必须存在一条本地队列，
并且有人在驱动它。三个后端的真实情况并不一样：

| | 本地队列归谁所有 | 谁驱动它 | 作用域是什么 |
|---|---|---|---|
| **tokio** | **本线程的 `thread_local!`**（`Rc<LocalSet>`） | `run_until` 或 `block_on_local`（后者还负责 park 等待） | 那条队列的别名（`!Send`、可 `Clone`、幂等） |
| **smol** | **本线程的 `thread_local!`**（`Rc<LocalExecutor>`） | `run_until` 或 `block_on_local` | 那条队列的别名（`!Send`、可 `Clone`、幂等） |
| **compio** | 运行时实例自己（线程绑定，不可分离） | 运行时自己 tick（`run_until` ≡ await）；`block_on_local` 自己跑那套 tick 循环 | 那份运行时的别名；**对齐基准** |

```rust
scope.spawn_local(async move { *rc });   // 投递点
scope.run_until(async { .. }).await;     // 异步驱动点
scope.block_on_local(f);                 // 同步等待，同时持续驱动队列（不借运行时阻塞原语）
```

好处有五条：

1. **不会串**：作用域只能从运行时值取得，且它就是**本线程那条队列**——不存在
   「作用域与运行时无关」或「同一线程好几条队列」的组合；
2. **前提编译期可见**：没写 `SPAWN_LOCAL` 位就取不到作用域；
3. **三个后端对齐同一套语义**：`Clone` 只增加别名、同一线程上多次取得拿到同一条、
   类型 `!Send`（tokio / smol 靠 `thread_local!` 做到，compio 本来如此——它是基准）；
4. **任务与句柄解绑**：队列归本线程而不是归 `JoinHandle`，因此 `detach()` 之后本地
   任务照常被驱动（这正是 `smux_v1` 的读 / 写循环所依赖的语义）；
5. **阻塞与驱动分清了两条路**：需要「同步等待 + 驱动本地队列」时用**作用域的**
   `block_on_local`（后端各按自己的运行时性质实现：tokio / smol 是「驱动队列 + park」，
   compio 是自己 tick）；需要「只等待、不驱动队列」时用**运行时值的** `block_on`。
   前者可以在本地队列的驱动栈内调用，后者在 tokio 上不行——两条路的差别写在方法文档里。

业务代码对本地投递只依赖 `S: TrLocalScope`（泛型于作用域），切换后端零改动。

**为什么队列不放在运行时值里**：tokio 的 `Handle` 本来 `Send + Sync`，塞进一个
`Rc<LocalSet>` 之后整个值就不能跨线程传了；而「可共享的把手」与「线程独占的队列」
本就该分开（实测见 [local-scope-vs-runtime-20261006-1050.md](dev-notes/local-scope-vs-runtime-20261006-1050.md)）。

### 虚拟时钟：测试自己推进时间

tokio 的 `test-util` 能在 `start_paused` 下把 `sleep(1h)` 变成瞬间完成；compio / smol
没有这个能力，于是「带虚拟时间的测试」只能写在 tokio 上。`abs_art-mock_clock` 把这项
能力做成**与后端无关的一份实现**，三个后端各以**可选 feature** `mock-clock` 接入：

```rust
use abs_art_mock_clock::{ManualClock, ManualTime};

let clock = ManualClock::new();
let scope = abs_art_smol::current().local_scope();
let value = ManualTime::new(abs_art_smol::current(), clock.clone());

let started = std::time::Instant::now();
// `block_on_advancing` 是各后端在 `mock-clock` feature 下的**固有方法**，不是
// `TrLocalScope` 契约的一部分——契约里的阻塞入口只有 `block_on_local`，它不推进虚拟时间。
scope.block_on_advancing(&clock, async {
    value.delay(core::time::Duration::from_secs(3_600)).await; // 虚拟一小时
    assert_eq!(value.now().as_millis(), 3_600_000);
});
assert!(started.elapsed() < core::time::Duration::from_secs(1)); // 真实时间几乎为零
```

推进参考 `tokio::time::advance` 做成 **future**（`rt.advance(d).await`）：推进不是纯状态写入，
被唤醒的任务需要有机会被调度；冻结用 `pause()` / `resume()`。

| 后端 | 驱动 | 「空闲」判据 |
| --- | --- | --- |
| smol | `smol::block_on(Supervisor)` | `LocalExecutor::try_tick()` |
| compio | `Runtime::block_on(Supervisor)` | `Runtime::run()` |
| tokio | `Handle::block_on(LocalSet::run_until(Supervisor))` | 不需要（`block_on` 自己驱动任务） |

三点设计要点：

1. **能力在 `abs_art`，实现在本 crate**：`TrClock` 定义「什么是时刻」，
   **`TrMockClock: TrClock`** 定义「时刻可以被调用方推进」——它是 `TrClock` 的**特例**，
   所以也在基础 crate 里（业务代码只约束 `TrClock`，测试代码才约束 `TrMockClock`）。
   手动时钟状态、到期唤醒表、delay 与驱动才是本 crate 的实现；扩展点
   （`ManualClockApi` / `MockInstant` / 驱动闭包）也都在这里。
2. **装饰器而非改造后端**：`ManualTime<R>` 只把 `TrDelay` / `TrClock` / `TrTime` 换成手动
   时钟，其余能力（`spawn` / `spawn_blocking` / `block_on` / 身份）原样委托给 `R`——
   因此「时刻与计时器同源」天然成立，而本地队列仍从 `inner()` 上取。
3. **静默挂起变响亮失败**：驱动在「既没有就绪任务、也没有可推进的定时器」时 panic，
   而不是让测试永远挂着。

**注意**：`mock-clock` 不在任何后端的默认 features 里；`just test-mock-clock` 会把它在
四个 crate 上跑一遍。

### 两种用法对照

| 场景 | 写法 | 后端如何决定 |
|---|---|---|
| 业务库（不感知后端） | `use abs_art_bridge::...`，泛型于 `R: TrXxx` 或声明 `Runtime::<{能力}>` | 集成方在 Cargo.toml 选 feature |
| 二进制（拥有运行时） | 构造运行时值（`abs_art_bridge::current()` 或 `Runtime::<{..}>::current()`）并传给业务库 | 自己创建运行时 |

### 需要注意的两点

- **业务库必须用 `default-features = false`**：后端选择权留给集成方。库自己的测试要在
  `[dev-dependencies]` 里再依赖一次 bridge 并开一个后端（Cargo 不会把那次的 feature
  泄漏进下游构建，已实测）。
- **启用多个后端时必须显式声明默认后端**：`cargo test --workspace` 会把各成员的
  feature 取并集；若此后没有 `default-backend-*`，裸名 `Runtime` 会按优先级悄悄选中
  一个（很可能是错的那个，运行期才崩）。bridge 对这种情况直接 `compile_error!`。
- **二进制的运行时构造代码与所选后端绑定**：bridge 负责抽象"能力"，不负责替你创建 tokio/compio 运行时实例——`main.rs` 里创建运行时的那几行本来就该属于"拥有运行时"的一方。
- **compio 没有 `TrSpawnSend`**：它没有跨线程全局工作队列（`spawn` 投的是本线程运行时的队列），所以不实现该 trait。需要「三后端同一份代码」时，只约束共同子集，投递任务走本地作用域。

## 测试

```sh
cargo test --workspace        # 全部 crate 的测试
cargo run -p abs_art-demo     # 运行演示（默认 compio 后端；tokio 组见下）
just test-mock-clock          # 虚拟时钟：本 crate + 三后端开 `mock-clock` 各跑一遍
just demo                     # 跑 abs_art-demo 两组 cap smoke tests（tokio + compio）
just smoke                    # 跑跨后端 spawn_local 行为契约矩阵（见下）
```

### 跨后端 `spawn_local` 行为契约（`abs_art-smoke`）

`just smoke` 用**同一份**测试体在 tokio / compio / smol 上各跑四个用例：

| 用例 | tokio | compio | smol |
|---|---|---|---|
| A 句柄驱动（spawn 后 await `JoinHandle`） | ✅ | ✅ | ✅ |
| B 运行时驱动（宿主**不 poll 句柄**，先等循环自己回报） | ✅ | ✅ | ✅ |
| C `detach()` 后循环继续被调度 | ✅ | ✅ | ✅ |
| D 阻塞组合（`rt.block_on(scope.run_until(f))`：阻塞在值、驱动在作用域） | ✅ | ✅ | ✅ |

**这 12 格曾经有两格是红的。** 在本 crate 建立时（最早的**类型级** `spawn_local`），
smol 的 B、C 两格失败：那时 `abs_art-smol` 每次 `spawn_local` 都新建一个
`LocalExecutor` 并把它绑在 `JoinHandle` 上，句柄一旦 poll 不到或被 `detach()` 掉，
本地任务就再也推不动。把本地队列改成由**本线程**持有（`thread_local!`，作用域只是
别名）之后两格转绿。

> D 用例的形态换过两次，本表记录的是**当前**这一版。最初走 `TrLocalScope::block_on`；
> 那条能力在「作用域对齐 compio 语义」这一轮被删（它把「怎么阻塞」整个交给后端，tokio
> 上只能用 `block_in_place`，而后者在 `LocalSet` 内被 tokio 自己禁止），于是 D 改成
> 组合写法 `rt.block_on(scope.run_until(f))`——**本表测的就是这个组合**。
>
> 后来发现组合写法在**本地队列的驱动栈内**依然不可用（`block_in_place` 的限制没有消失），
> 而调用方（`abs_buff_stdio_adapt` 的同步 `Read` / `Write`）恰恰就在那个位置。为此新增了
> 独立能力 [`TrLocalScope::block_on_local`](abs_art/src/runtime.rs)：**不使用任何运行时
> 阻塞原语**，三端各自「驱动队列 + park」（tokio / smol）或「自己 tick」（compio）。
> 它的契约测试不在本 smoke 里，而在三个后端各自的 `local_scope` 单测中
> （tokio 那份必须用多线程运行时——`current_thread` 下外部 park 驱动的 `run_until` 不推进
> 队列，最小复现见 `mptp_rpc/.tmp/tokio_nested`）。

调研过程、三个运行时的源码级能力对比与改造决策见
[`dev-notes/spawn_local-20261002-1247.md`](dev-notes/spawn_local-20261002-1247.md)。

`abs_art-demo` 的 smoke tests 按后端分组（`examples/tokio_demo/` 与
`examples/compio_demo/`，每种 cap 组合一个 example）：

```sh
just demo-tokio                                             # tokio 组（--no-default-features --features demo-tokio）
just demo-compio                                            # compio 组（--no-default-features --features demo-compio）
cargo run -p abs_art-demo                                                # compio 后端跑 main（默认）
cargo run -p abs_art-demo --no-default-features --features demo-tokio     # tokio 后端跑 main
```

# develop

## how to test
Use `just test` to test on all the supported asynchronous runtimes.
This requires `just` installed in the environment.

See [just](https://github.com/casey/just/blob/master/README.md) for more information.
