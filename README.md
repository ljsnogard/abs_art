# abs_art

ABStraction of Asynchronous RunTime.

一个 workspace，把 tokio / smol / compio 三种异步运行时抽象成统一的接口：

```
abs_art           基础 crate：不依赖任何运行时，只提供 enum Runtime 与一组 trait
abs_art-tokio     tokio 组合 crate：block_on / delay / spawn_send / spawn_local / spawn_blocking
abs_art-compio    compio 组合 crate：同上
abs_art-smol      smol 组合 crate：同上
```

## 结构

- `abs_art`（基础）只包含 [`runtime.rs`](abs_art/src/runtime.rs)：
  - `enum Runtime { Compio, Smol, Tokio }`（运行时标签）；
  - trait：`TrBlockOn`、`TrSpawnSend`、`TrSpawnLocal`、`TrSpawnBlocking`、`TrDelay`。
  - 由于孤儿规则（trait 与类型都来自 `abs_art` 时无法在外部 crate 中为它实现 trait），
    每个组合 crate 都定义自己的本地 `Runtime` 类型并为它实现这些 trait。
- 三个组合 crate 各自把五个功能做成 **feature 开关**：`block_on`、`delay`、
  `spawn_send`、`spawn_local`、`spawn_blocking`（默认全部开启），
  内部共用 `join_handle` feature 提供 `JoinHandle` / `JoinError` 包装。

## 用法

按后端选择对应的 crate：

```toml
[dependencies]
abs_art-tokio = { path = "abs_art-tokio", default-features = false, features = ["block_on", "spawn_send"] }
```

```rust
use abs_art_tokio::Runtime;

// 必须在 tokio 运行时上下文内调用
Runtime::block_on(async {
    let handle = Runtime::spawn(async { 42 });
    assert_eq!(handle.await.unwrap(), 42);
});
```

## 测试

每个组合 crate 的测试按运行时分开（tokio / compio / smol 各自的 crate）：

```sh
cargo test -p abs_art          # 基础 crate（无测试，仅编译）
cargo test -p abs_art-tokio    # tokio 后端测试
cargo test -p abs_art-compio   # compio 后端测试
cargo test -p abs_art-smol     # smol 后端测试
# 或一次跑完：
cargo test --workspace
```

# develop

## how to test
Use `just test` to test on all the supported asynchronous runtimes.
This requires `just` installed in the environment.

See [just](https://github.com/casey/just/blob/master/README.md) for more information.
