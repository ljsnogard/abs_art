# `CLOCK` 能力位 + compio 的 `SPAWN_SEND` 静态失败

日期：2026-10-06 13:05
分支：`feat/abs_art-runtime`
性质：**实施记录**（两件事都已落地并全绿）

---

## 1. `CLOCK` 能力位

### 1.1 位分配（6 位）

| 位 | 常量 | 值 |
| --- | --- | --- |
| 0 | `BLOCK_ON` | 1 |
| 1 | `DELAY` | 2 |
| 2 | `SPAWN_SEND` | 4 |
| 3 | `SPAWN_LOCAL` | 8 |
| 4 | `SPAWN_BLOCKING` | 16 |
| **5** | **`CLOCK`** | **32** |

`FULL` 由 **31 → 63**；`bit_assignment_is_stable` 加了 `assert_eq!(CLOCK, 32)`。
`impl_has!` 表由 5×16 = 80 涨到 **6×32 = 192** 个 impl，列表由脚本按「掩码含该位」生成
（`abs_art_runtime_probe/p1_orphan/z_caps6/` 是独立复验）。

### 1.2 门控

- `impl TrClock for Runtime<CAPS>` → `where [(); CAPS]: HasClock`
- `impl TrTime for Runtime<CAPS>` → `where [(); CAPS]: HasDelay, [(); CAPS]: HasClock`

**直接后果**：因为 `TrTime: TrDelay + TrClock`（同源约束），**要 `interval` / `timeout`
就必须同时声明 `CLOCK`**。这不是实现细节，而是那条结构约束的必然结果——已在三后端
与 caps 文档里写明，并各留一条 `compile_fail` 钉住。

### 1.3 一个「假通过」的教训（我的错）

我加位时**漏了 `abs_art/src/lib.rs` 的根重导出**（`CLOCK` / `HasClock` 只在 `caps` 模块里）。
后果不只是写法不便：`caps.rs` 里那条 `compile_fail` doctest 写的是
`use abs_art::{DELAY, HasClock};`，于是它**因为 unresolved import 而「通过」**——一个
把 bug 掩盖成绿灯的空测试。

修法：补上根重导出，并独立复验失败原因已变成真正的 trait bound：

```text
error[E0277]: the trait bound `[(); 2]: HasClock` is not satisfied
```

## 2. compio 的 `SPAWN_SEND` 静态失败

### 2.1 机制（实测出来的一条硬约束）

`abs_art-compio/src/caps.rs`：

```rust
pub const FULL: usize = ABS_ART_FULL & !SPAWN_SEND;   // = 59

#[diagnostic::on_unimplemented(
    message = "compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力",
    label   = "请从 CAPS 中去掉 `abs_art::SPAWN_SEND`；compio 的完整能力集是 `abs_art_compio::FULL`",
    note    = "compio 的执行器是线程本地的……要投递任务请用 `Runtime::local_scope()` 的 `spawn_local`。"
)]
pub trait CompioCaps_ {}
// 0..=63 中所有 m & SPAWN_SEND == 0 的掩码（32 个，宏生成）
```

并把 `[(); CAPS]: CompioCaps_` 写进 **`Runtime` 的类型定义**：

```rust
pub struct Runtime<const CAPS: usize = FULL>
where
    [(); CAPS]: CompioCaps_,
{ .. }
```

**关键**：`#[diagnostic::on_unimplemented]` 只挂在 impl 上时 rustc 走 `E0599`
（"trait bounds were not satisfied"），**不会**输出那段人话文案；必须挂在类型定义上才生效。
这条是实测结论（子代理给了两种落点的错误原文对照），不是风格选择。

### 2.2 效果（我自己复现）

```text
error[E0277]: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力
   |  let _value = abs_art_compio::Runtime::<{ abs_art::FULL }>::current();
   |               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |  请从 CAPS 中去掉 `abs_art::SPAWN_SEND`；compio 的完整能力集是 `abs_art_compio::FULL`
   = help: the trait `CompioCaps_` is not implemented for `[(); 63]`
```

`Runtime::<{ SPAWN_SEND }>`（=4）同样触发。覆盖入口：类型定义本身、`current` /
`with_runtime` / `retag`（堵住「换标签漂白」）/ `tag` / `runtime` / `local_scope`、
`Clone`，以及 `TrBlockOn` / `TrDelay` / `TrClock` / `TrTime` / `TrSpawnBlocking` /
`TrAsyncRuntime` 的 impl。

### 2.3 我又给错了一份掩码列表（子代理纠正）

我给子代理的 32 个掩码里混进了 `60..=63`（含 `SPAWN_SEND`）、漏了 `16..=19`
（只声明 `SPAWN_BLOCKING` 的合法组合）。子代理按我**写明的判据** `m & SPAWN_SEND == 0`
实现，并复核了集合恰好等于 `{m ∈ 0..=63 | m & 4 == 0}`（32 个，已由我独立验证）。
教训与 §1.3 同类：**判据比列举可靠**。

## 3. 下游适配

| crate | 改动 |
| --- | --- |
| `abs_art-tokio` / `abs_art-smol` | 加 `pub const FULL: usize = abs_art::FULL;`（数值同 63，语义是「本后端的完整能力集」）；`TrClock`/`TrTime` 门控改 `HasClock`(+`HasDelay`)；各补一负一正 doctest |
| `abs_art-compio` | 自己的 `FULL = 59`；`CompioCaps_` 静态断言（见 §2）；`TrClock`/`TrTime` 门控 |
| `abs_art-bridge` | 裸 `FULL` 改为**默认后端**的完整能力集（当前 = compio 的 59）；新增具名 `TokioFull` / `CompioFull` / `SmolFull`，并集构建下也精确；再导出 `CLOCK` / `TrMockClock` |
| `abs_art-demo` | `DelayRt = Runtime<{ DELAY \| CLOCK }>`（`now()`/`timeout` 需要）；`FullRt`/`current()` 按分组用 `TokioFull`/`CompioFull`；compio 的 compile_fail 改为「构造点 `E0277`」（原 `E0599`）；文档「五个能力位」→ 六个 |
| `abs_art-smoke` | **无需改动**（用 `current()` = 本后端完整集，已含 `CLOCK`） |

## 4. 验证（全部实跑）

| 项 | 结果 |
| --- | --- |
| `cargo test --workspace` | **24 个目标全 ok、0 失败** |
| `just test` | **EXIT=0**（含 compio 组 demo） |
| `just test-mock-clock` | **11 个目标全 ok、0 失败** |
| 三后端 `cargo test` | tokio 22+9+2、compio 29+16+10、smol 33+9+2 |
| `just demo`（14 示例 × 2 组） | 全通；tokio 组 `tag=Tokio`、compio 组 `tag=Compio` |
| clippy（workspace + 三后端开 `mock-clock`） | 各 crate **0 代码告警** |
| `cargo doc --workspace --no-deps` | **0 断链** |
| compio 静态失败 | 我独立复现（§2.2 原文） |

## 5. 留作后续

1. **smoke 契约**：把「虚拟时间」纳入契约矩阵（3 后端 × 1 格）。
2. `embedded-timers` 的 `Instant64` 作为可选 `MockInstant` 实现。
3. `abs_art-mock_clock` 的 no_std 变体。
4. 裸 `FULL` 在 bridge 里现在是「默认后端」的语义——若日后有人把默认后端换成 tokio，
   下游写裸 `FULL` 的代码会跟着变数值；具名别名不受影响（已写进 bridge 文档）。
