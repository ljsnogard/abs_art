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
where R: TrSpawnSend<...> + ...             // 泛型参数一路穿透所有代码
```

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
// 业务库 lib.rs —— 没有任何泛型参数穿透
use abs_art_bridge::{BLOCK_ON, SPAWN_SEND, Runtime, TrBlockOn, TrSpawnSend};

/// 业务库声明的能力组合：只需要 block_on + spawn_send 两种能力
pub type CapRt = Runtime<{ BLOCK_ON | SPAWN_SEND }>;

/// 业务函数：spawn 一个任务计算 x * 2，再 block_on 等待结果
pub fn double_via_runtime(x: i32) -> i32 {
    <CapRt as TrBlockOn<_>>::block_on(async move {
        let handle = <CapRt as TrSpawnSend<_>>::spawn(async move { x * 2 });
        handle.await.unwrap()
    })
}
```

`Runtime::<{ BLOCK_ON | SPAWN_SEND }>` 就是那个"能力声明"：这个类型**必然**实现 `TrBlockOn` 与 `TrSpawnSend`，并且**必然不**实现其它能力——比如在这里调用 `spawn_local`，编译器当场报错（Tag 严格模式）。

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
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt.block_on(async { my_business_lib::double_via_runtime(21) });
    assert_eq!(out, 42);
}
```

想换 compio / smol？改 `backend-tokio` → `backend-compio` / `backend-smol`，并按需调整 `main.rs` 里创建运行时的代码。**业务库一行都不用动。**

## 依赖结构与原理

### Workspace 结构

```
abs_art            基础 crate：不依赖任何运行时
                    ├─ enum Runtime（运行时标签）
                    ├─ trait：TrBlockOn / TrSpawnSend / TrSpawnLocal /
                    │          TrSpawnBlocking / TrDelay / TrAsyncRuntime / TrJoinHandle
                    │          （TrJoinHandle::detach：smol/compio 原生支持，
                    │           tokio 无原生 detach，drop 句柄即 detach——语义等价）
                    └─ caps：能力位掩码（BLOCK_ON / DELAY / SPAWN_SEND /
                              SPAWN_LOCAL / SPAWN_BLOCKING）与类型级标记

abs_art-tokio      tokio 后端：Runtime<const CAPS> 实现全部 trait
abs_art-compio     compio 后端：同上
abs_art-smol       smol 后端：同上

abs_art-bridge     桥接：backend-tokio / backend-compio / backend-smol 三选一
                   把 Runtime 与全部 trait / 能力常量重导出给业务代码

abs_art-demo       演示：业务库（零泛型穿透）+ 二进制（选后端）
                    examples/ 下按后端分组（tokio_demo / compio_demo）的
                    每种 cap 一个 smoke test（features：demo-tokio / demo-compio）
```

依赖关系（业务库只碰 bridge）：

```
业务库 ──► abs_art-bridge ──► abs_art-tokio / abs_art-compio / abs_art-smol（三选一）
                     └──────► abs_art（基础）
```

### 为什么是零开销

1. **能力检查发生在编译期**：`CAPS` 是 const 位掩码，`[(); CAPS]: HasBlockOn` 这类类型级标记在编译期被求解；`Runtime` 是零大小类型（ZST），`current()` 是 `const fn`——运行期没有任何能力相关的数据结构。
2. **调用是静态分发**：`<CapRt as TrBlockOn<_>>::block_on(...)` 在编译期被单态化为直接调用 tokio/compio/smol 的 API。**没有 vtable、没有 `Box`、没有 downcast、没有动态分发**——最终机器码与手写后端调用等价。
3. **后端 crate 的五个功能（`block_on` / `delay` / `spawn_send` / `spawn_local` / `spawn_blocking`）是 feature 开关**：按需编译，不用的代码不进产物。

对比其它方案：运行时注入（log 风格）需要类型擦除 + 装箱 + downcast，每次调用都有开销；泛型穿透需要把 `R` 参数写进每个函数签名。abs_art 用"能力声明"把两者都省掉了——零开销，且签名干净。

### 两种用法对照

| 场景 | 写法 | 后端如何决定 |
|---|---|---|
| 业务库（不感知后端） | `use abs_art_bridge::...`，声明 `Runtime::<{能力}>` | 集成方在 Cargo.toml 选 feature |
| 二进制（拥有运行时） | 直接依赖后端 crate / 或 bridge feature | 自己创建运行时 |

### 需要注意的两点

- **业务库必须用 `default-features = false`**：后端选择权留给集成方；若多个库各自选了不同后端，会触发 bridge 的编译期冲突检查（响亮报错，而不是静默错误行为）。
- **二进制的运行时构造代码与所选后端绑定**：bridge 负责抽象"能力"，不负责替你创建 tokio/compio 运行时实例——`main.rs` 里创建运行时的那几行本来就该属于"拥有运行时"的一方。

## 测试

```sh
cargo test --workspace        # 全部 crate 的测试
cargo run -p abs_art-demo     # 运行演示（业务库 + tokio 后端）
just demo                     # 跑 abs_art-demo 两组 cap smoke tests（tokio + compio）
```

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
