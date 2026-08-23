# abs_art

ABStraction of Asynchronous RunTime.

一个 workspace，把 tokio / smol / compio 三种异步运行时抽象成统一的接口：

```
abs_art           基础 crate：不依赖任何运行时，只提供 enum Runtime、能力 trait 与能力标签（caps）
abs_art-tokio     tokio 组合 crate：block_on / delay / spawn_send / spawn_local / spawn_blocking
abs_art-compio    compio 组合 crate：同上
abs_art-smol      smol 组合 crate：同上
abs_art-bridge    桥接 crate：通过 Cargo.toml 的 backend-* feature 选择后端
abs_art-demo      演示：业务库零泛型穿透 + 二进制选后端
```

## 结构

- `abs_art`（基础）：
  - `enum Runtime { Compio, Smol, Tokio }`（运行时标签）；
  - trait：`TrBlockOn`、`TrSpawnSend`、`TrSpawnLocal`、`TrSpawnBlocking`、`TrDelay`、
    `TrAsyncRuntime` / `TrJoinHandle`（句柄）；
  - [`caps`](abs_art/src/caps.rs)：能力位掩码（`BLOCK_ON` / `DELAY` / `SPAWN_SEND` /
    `SPAWN_LOCAL` / `SPAWN_BLOCKING`）与类型级标记，供编译期能力检查。
  - 由于孤儿规则（trait 与类型都来自 `abs_art` 时无法在外部 crate 中为它实现 trait），
    每个组合 crate 都定义自己的本地 `Runtime<const CAPS>` 类型并为它实现这些 trait。
- 三个组合 crate 各自把五个功能做成 **feature 开关**（默认全部开启），
  内部共用 `join_handle` feature 提供 `JoinHandle` / `JoinError` 包装。
- `abs_art-bridge`：通过 `backend-tokio` / `backend-compio` / `backend-smol`
  **三选一**的 feature 把 `Runtime` 重导出给业务代码。

## 用法

### 二进制层：具体用法（选择后端 = 选择 crate）

```toml
[dependencies]
abs_art-tokio = { path = "abs_art-tokio" }
```

```rust
use abs_art_tokio::Runtime;

// 必须在 tokio 运行时上下文内调用
Runtime::block_on(async {
    let handle = Runtime::spawn(async { 42 });
    assert_eq!(handle.await.unwrap(), 42);
});
```

### 业务库层：Tag 用法（通过 `abs_art-bridge`，零泛型穿透、零运行时开销）

业务库只依赖 `abs_art-bridge`（后端由集成方在 Cargo.toml 里选定）：

```toml
# 集成方（通常是二进制）的 Cargo.toml：
[dependencies]
abs_art-bridge = { path = "abs_art-bridge", features = ["backend-tokio"] }
```

```rust
use abs_art_bridge::{BLOCK_ON, SPAWN_LOCAL, Runtime, TrBlockOn, TrSpawnLocal};

// 声明所需能力；未声明（或后端不支持）的能力在编译期报错（Tag 严格模式）
let rt = Runtime::<{ BLOCK_ON | SPAWN_LOCAL }>::current();
let _ = rt;
// <Runtime<{ BLOCK_ON | SPAWN_LOCAL }> as TrBlockOn<_>>::block_on(async { 1 });
// <Runtime<{ BLOCK_ON | SPAWN_LOCAL }> as TrSpawnLocal<_>>::spawn_local(async { 2 });
```

切换后端 = 改集成方 Cargo.toml 里的 `backend-*` feature（并相应调整二进制的
运行时创建代码），**业务库代码零改动**。

## 测试

```sh
cargo test --workspace        # 全部 crate（bridge 按 workspace 选定的后端运行）
cargo test -p abs_art-tokio   # 单个后端
cargo run -p abs_art-demo     # 运行演示二进制
```

# develop

## how to test
Use `just test` to test on all the supported asynchronous runtimes.
This requires `just` installed in the environment.

See [just](https://github.com/casey/just/blob/master/README.md) for more information.
