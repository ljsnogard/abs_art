# 本地队列应该挂在 `LocalScope` 上，不是运行时值上

> **后记（2026-10-06 14:20）：本文 §2 的「值持有队列」形状已被取代。** 本日裁决把队列收回
> **线程本地**：tokio / smol 用 `thread_local!`，作用域只是那条队列的别名（`Clone` 即别名、
> 同一线程上 `local_scope()` 幂等），compio 的现有形状成为**对齐基准**；
> `TrLocalScope::block_on` 同时从 trait 上删除。因此本文 §2 的表格、§3.5 与 §5 描述的是
> 当时的中间形态。最终裁决、实测与代价见 `local-scope-thread-local-20261006-1420.md`。

日期：2026-10-06 10:50
分支：`feat/abs_art-runtime`（值化改造已落地：`runtime-value-20261006-1022.md`）
性质：**设计裁决 + 实测**（本轮不改公开 API，只加探针与结论）
探针：`abs_art_runtime_probe/p7_queue_owner/`（1 个 crate，3 条实验）+ 复用
`p6_static_runtime/` 的 Send/Sync 证据。

---

## 0. 结论

**提案成立：本地队列不该是运行时值的一部分。**

但前提要修正一处：**不是「只有 tokio 有这个问题」**——smol 的 `LocalExecutor` 同样是
调用方创建、调用方驱动的独立对象，与 tokio 同类；真正「不可分离」的是 compio：
它的执行器就在运行时实例里，而运行时本身是线程绑定的。

| 后端 | 队列在哪 | 能不能与「运行时」分离 | 实验证据 |
| --- | --- | --- | --- |
| tokio | 调用方的 `Rc<LocalSet>` | **能**（`Handle` 与 `LocalSet` 是两个对象） | 在 B 的上下文里驱动 `v_a` 的队列，任务照样跑 → 队列随**值**走 |
| smol | 调用方的 `Rc<LocalExecutor>` | **能** | 只驱动 `v_b` 不跑 `v_a` 的任务；驱动 `v_a` 才跑 → 队列随**值**走 |
| compio | 运行时实例自己的 `Rc<Executor>` | **不能** | 在 B 的上下文里用 `v_a` 投递 → 不跑；回到 A 的上下文 → 跑 → 队列钉在**运行时实例**上 |

`[实测]`（`p7_queue_owner`，同线程先后起两个运行时 A、B，用 A 的值 `v_a` 投递任务）：

```text
[tokio]  在 B 的上下文里驱动 v_a 的队列 → 任务执行 = true
[compio] v_a 投递的任务：在 B 的上下文被执行 = false；回到 A 的上下文 = true
[smol]   v_a 的任务：只驱动 v_b 之后 = false；驱动 v_a 之后 = true
```

`[源码事实]` compio：`Runtime::spawn_at` → `self.executor.spawn_at(..)`（执行器是实例字段），
`CURRENT_RUNTIME` 只是 `scoped_thread_local!`（供自由函数与计时器取「当前」运行时）。
所以 compio 的「本地队列」= 运行时的队列，值就是那份运行时的把手（`!Send`）。

## 1. 当前实现错在哪（症状，都可复现）

1. **tokio 的运行时值被迫变成 `!Send`**。它内部是 `Handle`（`Send + Sync`）+ `Rc<LocalSet>`
   （`!Send`），于是「全局 spawn 的能力」不能跨线程传——`[实测]` 关掉 `local_scope`
   后同一个 `static OnceLock<Runtime<FULL>>` 就能编译（P6），开启后是
   `E0277: shared static variables must have a type that implements Sync`。
   **队列是唯一障碍。**
2. **两种生命周期被强行绑在一起**：句柄可以共享、可以长期活着；队列必须绑定线程、
   必须由持有者驱动。现在「值亡 ⇒ 队列亡 ⇒ 未完成任务被析构」——`[实测]` P6：
   `drop(value)` 之后挂起任务的哨兵当场析构（计数 1）。队列本可以活得比某个把手更长
   （一个线程一条队列，值只是临时把手）。
3. **抽象上出现「同一 trait 挂在不同语义的宿主上」**：compio 侧 `TrLocalScope` 落在
   runtime 值上是对的（队列就是它的），tokio/smol 侧落在值上则是把两个对象揉成一个。
   库侧从类型上读不出「这个宿主是不是线程独占」。
4. **一个运行时上开多个独立作用域很别扭**：只能造多个不同 `CAPS` 的 `Runtime` 值
   （`with_handle`），而作用域本该是同类可多份的东西。

## 2. 修正形状（保留值化，只把队列剥出来）

```rust
// abs_art：TrLocalScope 回到「线程独占的值」上；block_on 也回来
// （宿主类型与 Runtime 不同，不再有歧义，见 runtime-value 笔记 §2.2）
pub trait TrLocalScope {
    type Handle<T>: TrJoinHandle<T> where T: 'static;
    fn spawn_local<F>(&self, f: F) -> Self::Handle<F::Output>
    where F: Future + 'static, F::Output: 'static;
    fn run_until<F>(&self, f: F) -> impl Future<Output = F::Output> where F: Future;
    fn block_on<F>(&self, f: F) -> F::Output where F: Future;
}
```

| 后端 | `Runtime<CAPS>` 持有什么 | `Send`? | `LocalScope` 是什么 | 取得方式 |
| --- | --- | --- | --- | --- |
| tokio | `handle_: tokio::runtime::Handle` | **`Send + Sync`** | `local_: Rc<LocalSet>`（`!Send`） | `Runtime::<CAPS>::local_scope()`，要求 `[(); CAPS]: HasSpawnLocal` |
| smol | 无（全局执行器进程级，值是 ZST） | **`Send + Sync`** | `local_: Rc<LocalExecutor<'static>>`（`!Send`） | 同上 |
| compio | `rt_: compio::runtime::Runtime`（线程绑定，compio 自身如此） | `!Send`（compio 决定） | **ZST**（队列归当前运行时） | 同上 |

要点：

- **caps 位重新门控「取得作用域」这条路**（原设计的形状），`SPAWN_LOCAL` 仍然是「写在
  代码上的声明」；`TrLocalScope` 的实现从 `Runtime` 移到 `LocalScope` 上。
- **构造入口收窄**：`LocalScope` 只能从运行时值取得（构造函数不公开），从类型上消除
  原设计那种「作用域值与运行时无关」的串。（原设计的 `LocalScope::new()` 是公开的，
  这正是当时需要给 `SPAWN_LOCAL` 位写「拦不住真想用的人」那段免责声明的原因。）
- **compio 的 `LocalScope` 是 ZST**，`spawn_local` → 当前运行时的队列、`run_until` ≡
  直接 await——这与它「队列不可分离」的事实一致，也是原设计的形状（已被实测证明忠实）。
- 上一轮的命名结论继续有效：`LocalScope::clone`（共享同一条队列）与
  「同运行时 + 新队列」（`Runtime::with_local_queue()` 之类）要分开命名。

## 3. 代价与待定

1. **库侧可能重新需要两个参数**（运行时值 `R` + 作用域值 `S`）。原设计的缓解办法可复用：
   给 `LocalScope` 也实现 `TrDelay` / `TrClock` / `TrTime`（委托给环境计时器），
   于是 `S: TrLocalScope + TrTime` 一个参数就够。**这条需要裁决**：
   - 选「作用域也带计时能力」→ 库侧仍一个参数，但 `LocalScope: TrTime` 是建模折衷
     （计时源其实来自环境，不是作用域）；
   - 选「纯作用域」→ 库侧写两个参数（`rt: &R`, `scope: &S`），建模更干净。
2. **公开 API 再次变动**：`LocalScope` 类型回归、`TrLocalScope::block_on` 回归、
   `Runtime::local_scope()` 回归。三后端 + bridge/demo/smoke 需要再适配一轮。
3. **compio 侧的一个既存（且静默）的坑**：持有 A 运行时的值、在 B 的上下文里投递，
   任务**永远不跑**（实测 `false`）。拆出 `LocalScope` 后这个坑仍然存在，但会变得**可见**
   （你手上拿的是「另一个运行时的作用域」），而不是藏在「运行时值自己就是队列」的说法里。

## 3.5 追加裁决（同日，两条精度修正）

1. **compio 不实现 `TrSpawnSend`。** 它没有跨线程全局工作队列——`Runtime::spawn_at` →
   `self.executor.spawn_at`，投的是**本线程**运行时的队列；假装成 `TrSpawnSend`
   会让「三后端共同能力」这个约束失真。删掉该实现之后，共用业务代码只能建立在真实
   的共同子集上（`TrBlockOn` + `TrDelay`/`TrTime`/`TrClock` + `TrSpawnBlocking` +
   `TrLocalScope`），**这比让 compio 顶着一个它没有的能力更精确**。
   （`TrSpawnBlocking` 保留：compio 的 `spawn_blocking` 确实把闭包放到别的线程上跑。）
2. **`LocalScope` 不实现 `TrDelay` / `TrClock` / `TrTime`。** 计时与时刻与「在哪个线程
   调度」无关，它们属于运行时值；调用者从 `rt` 上取。作用域只回答一个问题：
   「`!Send` 任务投到哪、由谁驱动」。代价是库侧可能写两个参数（`R` + `S`），这是
   建模干净换来的——见 §3.1 的两条路，此裁决选了「纯作用域」。

## 4. 建议

**按本方案改。** 它消除的是一条真实的设计错误（把线程独占资源塞进可共享把手），
代价是公开 API 再动一轮——而分支本来就是这个用途。要动手的话，工作量与上一轮同量级：
`abs_art`（trait 形状 + 文档/doctest）→ 三后端（值拆两半 + 各自的 `LocalScope`）→
bridge/demo/smoke 适配 → 全 workspace 测试。

**唯一需要先定的是 §3.1**：`LocalScope` 要不要顺手实现 `TrDelay/TrClock/TrTime`
（库侧一个参数 vs 两个参数）。

---

## 5. 实施记录（同日完成，分支 `feat/abs_art-runtime`）

### 5.1 改了什么

| 位置 | 改动 |
| --- | --- |
| `abs_art/src/runtime.rs` | `TrLocalScope` 宿主改为「独立作用域值」，**恢复 `block_on`**；模块文档重写为「两个概念两张皮」（运行时值 vs 线程独占作用域），并记下曾经合并的代价；`TrBlockOn` 文档去掉「同时驱动本地队列」 |
| `abs_art/src/caps.rs` | `SPAWN_LOCAL` 的门控点改为「`Runtime::<CAPS>::local_scope()` 这唯一入口」 |
| `abs_art-tokio` | `Runtime<CAPS>{handle_}`（**`Send + Sync`**，加了编译期断言用例）；`LocalScope{handle_, Rc<LocalSet>}`（`Clone` 共享队列、`block_on` 走 `block_in_place`）；`local_scope()` 门控 `HasSpawnLocal`；`Runtime::block_on` 不再驱动队列 |
| `abs_art-smol` | `Runtime<CAPS>` 退化为 **ZST 标记**（`Send + Sync`）；`LocalScope{Rc<LocalExecutor>}`；`TrSpawnSend` 保留（`smol::spawn` 是真正的进程级多线程执行器） |
| `abs_art-compio` | `Runtime<CAPS>{rt_}` 原样（compio 的运行时本身线程绑定）；`LocalScope{rt_}`（队列不可分离，`run_until` ≡ await）；**删除 `impl TrSpawnSend`**，`spawn_send.rs` 改成「为什么不实现」的文档模块 + 2 条 `compile_fail` 钉住 |
| `abs_art-bridge` | `LocalScope` 重导出回归（裸名按 cfg 优先级 + `TokioLocalScope`/`CompioLocalScope`/`SmolLocalScope`）；文档示例改为「值 → 作用域」 |
| `abs_art-demo` | 共同子集的业务函数（不含 `TrSpawnSend`）；本地投递经 `rt.local_scope()`；compio 组的 `cap_spawn_send` 改成**反向演示**（compio 不实现 `TrSpawnSend`，应改用本地作用域） |
| `abs_art-smoke` | 本地投递契约改为泛型于**作用域** `S: TrLocalScope`；时间契约仍泛型于**运行时值** `R: TrTime`；**新增第 5 条契约**「计时与时刻来自运行时值、不是作用域」（3 后端 × 1）+ 一条按后端分组的编译期用例（只有 tokio/smol 断言 `TrSpawnSend`） |
| `README.md` | 版本叙述改成「本版三轮收敛」的定型叙述；快速上手改成「值 + 作用域」两件套；写明 compio 没有 `TrSpawnSend` 的后果 |

### 5.2 验证（全部实跑）

| 项 | 命令 | 结果 |
| --- | --- | --- |
| 全 workspace | `cargo test --workspace` | **EXIT=0**（21 个测试目标、约 195 个测试结果全过） |
| 项目自带验收 | `just test` | **EXIT=0**，`abs_art-demo (tokio backend) OK: send=42, local=42` |
| demo 两组 | `cargo test -p abs_art-demo`（7+8+6）与 `--no-default-features --features demo-compio`（6+7+4） | 均 **EXIT=0** |
| smoke 矩阵 | `cargo test -p abs_art-smoke` | `spawn_local_contract` 13、`time_contract` 24、doctest 1，全过 |
| clippy | `cargo clippy --workspace --all-targets` | 各 crate 0 诊断（仅根 `Cargo.toml` 的 6 条既存 manifest 告警） |
| bridge 守卫 | 单后端 / 多后端无默认 / 多后端+一个默认 / 两个默认 | `0 / compile_error / 0 / compile_error`，符合预期 |
| 队列归属实验 | `abs_art_runtime_probe/p7_queue_owner`（已适配新 API） | `tokio: 在 B 的上下文驱动 s_a 的队列 → true`；`compio: B 里 false、回 A 里 true`；`smol: 只驱动 s_b false、驱动 s_a true` |

### 5.3 顺带修掉的两处

1. `abs_art/src/runtime.rs` 的 `TrJoinHandle::detach` 「已知限制」还写着「执行器随**运行时值**存活」——已改为「随作用域存活」。
2. `abs_art-tokio` 在关掉 `local_scope` 时 `use alloc::rc::Rc;` 未使用——已按 feature 门控。

### 5.4 留作后续（不在本轮范围）

- **`SPAWN_SEND` 位在 compio 上不再对应任何实现**：位仍可写（它是声明），但写了也拿不到 `spawn`。是否要在文档/错误信息里把这条讲得更直白，或干脆让 compio 的类型别名不推荐该位，未定。
- **`LocalScope` 的 `!Send` 与「构造函数不公开」没有编译期负向断言**（stable 写不出否定约束；构造函数私有只靠 `pub(crate)`）。
- **smoke 第 5 条契约是回归闸门而非证明**：类型系统没有负实现，若将来有人给作用域实现了 `TrTime`，该契约不会红（已在探针文档写明）。

---

## 6. 缺省后端定为 compio，并补齐「每后端一格」的桥接测试（同日追加）

### 6.1 裁决

`abs_art-bridge` 的缺省后端从 tokio 改为 **compio**（`abs_art-demo` 的缺省演示组随之
改为 `demo-compio`，与 bridge 缺省保持一致）。

### 6.2 写法必须是 `default-backend-compio`，不能是裸 `backend-compio`

差别只在「默认是谁」有没有**显式声明**，但后果是构建过不过：

| 场景 | `default = ["backend-compio"]` | `default = ["default-backend-compio"]` |
| --- | --- | --- |
| `cargo build -p abs_art-bridge`（默认） | ok | ok |
| **默认 + `--features backend-tokio`**（≈ `cargo test --workspace`：bridge 作为 workspace 成员的缺省 feature 与 demo 的 `bridge_tokio` 取并集） | **`compile_error!`**（并集里两个后端、却没有默认声明） | ok |

也就是说：缺省后端改了之后，**必须**把「默认」写成声明式 feature，否则 workspace 级
构建会踩到「多后端必须显式声明默认」这条守卫（守卫本身是对的，见
`runtime-static-proposal-20261006-1043.md` 的 P5 实测）。

`justfile` 与 README 里所有「默认 = tokio」的说明一并改掉；`demo-tokio` 配方补上了
`--no-default-features --features demo-tokio`（缺省换人之后不关缺省就选不到 tokio 组）。

### 6.3 桥接测试补成「每后端一格」

`abs_art-bridge` 原来只有 `tests_tokio_` 一个测试模块。现在三个：

| 模块 | 门控 | 验证 |
| --- | --- | --- |
| `tests_tokio_` | `all(test, feature = "backend-tokio")` | `tag() == Tokio`、`local_scope()` 可用、`!Send` 任务跑通、运行时值 `Send + Sync` |
| `tests_compio_` | `all(test, feature = "backend-compio")` | `tag() == Compio`、`local_scope()` 可用、`!Send` 任务经 `run_until` 跑通 |
| `tests_smol_` | `all(test, feature = "backend-smol")` | `tag() == Smol`、作用域 `block_on` 驱动任务、运行时值 `Send + Sync` |

**三个模块都用具名别名**（`TokioRuntime as Runtime` / `CompioRuntime as Runtime` /
`SmolRuntime as Runtime`），不用裸名——否则在「缺省 compio + 下游要 tokio」的并集构建里，
tokio 模块会拿到 compio 的类型（这正是 6.2 那个并集场景）。实测并集构建下 4 条用例
（compio 2 + tokio 2）全部运行并通过。

### 6.4 demo 侧的同源修正

`bridge_tokio` 与 `bridge_compio` 在 workspace 构建里是**同一个包**（feature 取并集），
所以 `abs_art_bridge::Runtime` 这种裸名会跟着 bridge 的缺省后端走，而 demo 自己的
`demo-*` 分组才是它真正想要的。因此：

- `abs_art-demo/src/lib.rs` 不再重导出 bridge 的裸名 `Runtime` / `LocalScope` /
  `current`，改为按分组 `pub use abs_art_bridge::{TokioRuntime as Runtime, …}`，并自带
  分组内的 `current()`（供 doctest 用）；
- 14 个示例的 `use bridge_{tokio,compio}::Runtime` 全部改成
  `TokioRuntime as Runtime` / `CompioRuntime as Runtime`。

### 6.5 验证（全部实跑）

| 项 | 结果 |
| --- | --- |
| `cargo test --workspace` | **21/21 目标 ok，0 失败** |
| bridge 三组测试 | 默认（compio）2 + 2 doctests；`--features backend-tokio` 4 用例（compio 2 + tokio 2）；`--features backend-smol` 1 + 2 doctests |
| `just demo`（14 个示例 × 两组） | 全通；tokio 组 `tag=Tokio`、compio 组 `tag=Compio` |
| `just test` | **EXIT=0**，`abs_art-demo (compio backend) OK: local=42, local_tasks=42` |
| clippy（workspace + demo 两组） | 各 crate 0 诊断 |
| 守卫矩阵 | 默认 ok / 默认+tokio ok / 多后端无默认 `compile_error` / 多后端+一个默认 ok / 两个默认 `compile_error` |
