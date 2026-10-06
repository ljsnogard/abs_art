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

> v0.3 起，`TrSpawnSend` / `TrSpawnBlocking` / `TrBlockOn` 不再带 `F` 类型参数：
> 被 spawn 的 future 类型是各自动**方法级泛型参数**。
> 于是一个 `R: TrSpawnSend` 约束就能覆盖该运行时上**所有**任务类型
> （包括调用点无法命名的 `async {}` 块），不必为每个 future 写一个
> `TrSpawnSend<F>`。上面「泛型穿透」的痛点依然存在，但约束数量不再随
> 任务种类增长。
>
> 同一版还把**本地投递**拆成了「声明位 + 值」两半：`TrSpawnLocal` 不再存在，
> 换代的是能力位 `SPAWN_LOCAL`（只负责声明）与 `TrLocalScope`（负责真正的投递与驱动）。
>
> **v0.4：运行时值化。** 所有能力 trait 的方法改收 `&self`，运行时从「类型标签」
> 变成「**值**」——本地队列、计时器、时刻源都由同一个值提供。于是 v0.3 的两处
> 「串」（独立作用域值、独立时钟配置）从结构上消失：「哪条队列 / 哪个时钟」由你
> 手上那个值回答，而不是靠约定。`LocalScope` 这个独立类型因此并入运行时值，
> 新增 `TrClock`（`TrTime: TrDelay + TrClock`，让「同源」成为类型约束）。
> 理由与代价见 [「本地投递：一个声明位 + 一个运行时值」](#本地投递一个声明位--一个运行时值)。

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
use abs_art_bridge::{TrBlockOn, TrSpawnSend};

/// 业务函数：拿到运行时**值**，spawn 一个任务计算 x * 2，再 block_on 等待结果
pub fn double_via_runtime<R>(rt: &R, x: i32) -> i32
where
    R: TrBlockOn + TrSpawnSend,
{
    rt.block_on(async move {
        let handle = rt.spawn(async move { x * 2 });
        handle.await.unwrap()
    })
}
```

v0.4 起**运行时是值**：能力都是 `&self` 方法（`rt.spawn(..)` / `rt.block_on(..)` /
`rt.delay(..)` / `rt.spawn_local(..)`），因此业务库只要泛型于「有这些能力的值」即可。

集成方若想**声明**「我只给这个库 BLOCK_ON | SPAWN_SEND」，就把值构造成对应能力位：
`Runtime::<{ BLOCK_ON | SPAWN_SEND }>::current()`——这个类型**必然**实现
`TrBlockOn` 与 `TrSpawnSend`，并且**必然不**实现其它能力（比如 `spawn_local`），
编译器当场报错。

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
        my_business_lib::double_via_runtime(&rt, 21)
    });
    assert_eq!(out, 42);
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
                    ├─ trait：TrLocalScope —— 本地投递与异步驱动入口
                    │          （spawn_local / run_until；阻塞驱动统一走 TrBlockOn）
                    └─ caps：能力位掩码（BLOCK_ON / DELAY / SPAWN_SEND /
                              SPAWN_LOCAL / SPAWN_BLOCKING）与类型级标记
                              ※ 位负责「声明」，且真正门控调用点（见下）

abs_art-tokio      tokio 后端：Runtime<const CAPS> 是**运行时值**
                   （持 Handle + Rc<LocalSet>），实现全部 trait
abs_art-compio     compio 后端：同上（运行时本身线程本地）
abs_art-smol       smol 后端：同上（本地队列为值持有的 Rc<LocalExecutor>）

abs_art-bridge     桥接：backend-tokio / backend-compio / backend-smol 可多选，
                   裸名 Runtime 默认哪个由 cfg 优先级 / 显式 default-backend-* 决定；
                   另给具名别名 TokioRuntime / CompioRuntime / SmolRuntime

abs_art-demo       演示：业务库（零泛型穿透）+ 二进制（选后端）
                    examples/ 下按后端分组（tokio_demo / compio_demo）的
                    每种 cap 一个 smoke test（features：demo-tokio / demo-compio）

abs_art-smoke      跨后端 spawn_local 行为契约冒烟测试（publish = false）
                    同一份测试体（泛型于运行时值 R: TrLocalScope）分别跑在三个
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
3. **运行时值本身很小**：tokio 版是「一个 `Handle` + 一个 `Rc<LocalSet>`」，没有任何按能力构造的数据结构；`spawn` / `delay` / `now` 的路径上只有一次直接调用。
3. **后端 crate 的五个功能（`block_on` / `delay` / `spawn_send` / `local_scope` / `spawn_blocking`）是 feature 开关**：按需编译，不用的代码不进产物。

对比其它方案：运行时注入（log 风格）需要类型擦除 + 装箱 + downcast，每次调用都有开销；泛型穿透需要把 `R` 参数写进每个函数签名。abs_art 用"能力声明"把两者都省掉了——零开销，且签名干净。

### 本地投递：一个声明位 + 一个运行时值

`spawn_local` 仍然是两半，**职责不同、不互相替代**——但 v0.4 起「值」的那一半换人了：

| | 回答的问题 | 载体 |
|---|---|---|
| 能力位 `SPAWN_LOCAL` | 你**声明**了没有？ | `Runtime<CAPS>` 的类型级标记 |
| 运行时值 | 你**拿到**了没有？ | 实现 `TrLocalScope` 的**运行时值本身** |

#### 位负责「声明」

位**拦不住**真想用的人——把位写上就够了。它的价值在于**强制显式**：想开始用本地投递，就必须先把这件事写下来，于是它必然出现在类型别名、diff 与 code review 里，也能被 `grep` 出来。最贴切的类比是 `unsafe`：任何人都会写，但**必须写**。

```rust
use abs_art_bridge::{Runtime, SPAWN_LOCAL, TrLocalScope};

// 声明：这次「升级」留了痕；且位**真的**门控调用点
let rt = Runtime::<{ SPAWN_LOCAL }>::current();
let handle = rt.spawn_local(async { 1u32 });   // ← 只有写了位才编译得过
```

对比 v0.3：那时位只门控「取作用域的入口」（`local_scope()`），而 `LocalScope::new()`
是公开入口、绕过声明依然可行。v0.4 把本地队列并进运行时值之后，**位直接门控
`rt.spawn_local(..)` 本身**——负向演示见 `abs_art-demo` 的 `compile_fail` 文档测试。

（仍然不是安全边界：任何人都可以写 `Runtime::<{ SPAWN_LOCAL }>`。位的价值是
「你必须写下来」，约束的是**意外**，不是**恶意**。）

#### 运行时值负责「事实」

`spawn_local` 除了「运行时支持」之外还有一条**环境前提**：必须存在一个本地队列，并且有人在驱动它。三个后端的真实情况并不一样：

| | 本地队列归谁所有 | 谁驱动它 |
|---|---|---|
| **tokio** | 运行时值（内部 `Rc<LocalSet>`） | 同一个值的 `run_until` / `block_on` |
| **smol** | 运行时值（内部 `Rc<LocalExecutor>`） | 同一个值的 `run_until` |
| **compio** | 运行时自己（线程本地） | 运行时在驱动期间自己推 |

这条前提**表达不进类型参数**：纯类型约束只能证明「你把位写对了」，证明不了「此刻真的有一条被驱动的队列」。v0.3 用「独立的作用域值」承担它，代价是「驱动谁」与「用哪个运行时」成了两件事；v0.4 让**运行时值本身**就是队列的持有者与驱动点：

```rust
rt.spawn_local(async move { *rc });   // 投递点
rt.run_until(async { .. }).await;     // 异步驱动点
rt.block_on(async { .. });            // 阻塞驱动点（同一次调用也驱动本地队列）
```

好处有四条：

1. **不会串**：同一个进程里存在两套运行时（例如测试二进制里 tokio 与 compio 并存）时，「哪条队列」由你手上那个值回答，不需要靠约定；
2. **前提编译期可见**：没写 `SPAWN_LOCAL` 位就没有 `spawn_local` 可调；
3. **三个后端如实表达自己**：tokio / smol 的值装着各自的本地队列，compio 的值是零大小（队列归运行时自己）；
4. **任务与句柄解绑**：队列随运行时值存活而不是随 `JoinHandle` 存活，因此 `detach()` 之后本地任务照常被驱动（这正是 `smux_v1` 的读 / 写循环所依赖的语义）。

业务代码只依赖 `R: TrLocalScope`（泛型于运行时值），切换后端零改动。

**代价**：tokio / smol 的运行时值因为持有 `Rc` 而是 `!Send`——本地队列本来就绑定线程。
需要跨线程使用全局能力时，在目标线程各构造一个值（tokio 提供
`Runtime::with_handle(handle)`，可以在上下文之外先把值准备好）。

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

## 测试

```sh
cargo test --workspace        # 全部 crate 的测试
cargo run -p abs_art-demo     # 运行演示（业务库 + tokio 后端）
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
| D `rt.block_on` 阻塞驱动（同时推本地队列） | ✅ | ✅ | ✅ |

**这 12 格曾经有两格是红的。** 在本 crate 建立时（v0.3 的类型级 `spawn_local`），
smol 的 B、C 两格失败：那时 `abs_art-smol` 每次 `spawn_local` 都新建一个
`LocalExecutor` 并把它绑在 `JoinHandle` 上，句柄一旦 poll 不到或被 `detach()` 掉，
本地任务就再也推不动。把本地队列改成由**作用域值**持有之后两格转绿。

调研过程、三个运行时的源码级能力对比与改造决策见
[`dev-notes/spawn_local-20261002-1247.md`](dev-notes/spawn_local-20261002-1247.md)。

`abs_art-demo` 的 smoke tests 按后端分组（`examples/tokio_demo/` 与
`examples/compio_demo/`，每种 cap 组合一个 example）：

```sh
just demo-tokio                                             # tokio 组（默认 features）
just demo-compio                                            # compio 组（--no-default-features --features demo-compio）
cargo run -p abs_art-demo --no-default-features --features demo-compio   # compio 后端跑 main
```

# develop

## how to test
Use `just test` to test on all the supported asynchronous runtimes.
This requires `just` installed in the environment.

See [just](https://github.com/casey/just/blob/master/README.md) for more information.
