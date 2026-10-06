# 作用域对齐 compio 语义：删掉 `TrLocalScope::block_on`，队列进 `thread_local!`

日期：2026-10-06 14:20
分支：`feat/abs_art-runtime`（回退点：`0320251` Add Clone for ManualTime and record the downstream gap）
性质：**设计裁决 + 实施记录**（本轮改动生产代码，含公开 API 变更）
探针：`abs_art_runtime_probe/ls_misuse_probe/`（4 个 bin）

---

## 0. 两条裁决（人类下达，原文要点）

1. **`TrLocalScope` 不能再声称支持 `block_on`**——「这个从 trait 上的根源就错了」，
   只能保留正确名字的 `run_until`（本轮采纳该名），并写清三个后端各自如何支持这条语义。
2. **`TrLocalScope` 一律对齐 compio 的语义**：它是一个 `thread_local` 对象，
   可以 `Clone` 但不能 `Send`；无论 `Clone` 多少次都只是 `thread_local!` 里那个对象的
   别名。同一线程上创建多个 `LocalSet` 这种 tokio 用法**显式不支持**——
   「我们只为兼容 tokio，不代表要完整支持 `LocalSet`」。

配套选择（同日确认）：`mock-clock` 的 `block_on_advancing` 保留为**各后端固有方法**并写明
它不属于 `TrLocalScope`；**不**新增 `reset()` / `with_scope()`，测试隔离由「用例各自起独立
线程」承担。

---

## 1. 为什么要删 `block_on`（因果链）

上一轮的结论「阻塞与驱动是两件事」只写进了文档，trait 上仍留着
`scope.block_on(f)`。本轮把三个后端在这条方法上的**真实语义**测了出来，它们不是同一种能力：

| 场景 | tokio | smol | compio |
| --- | --- | --- | --- |
| `TrBlockOn::block_on` 期间，同线程**全局任务**是否推进 | 是（迁移到替代 worker） | 不 tick 执行器（但全局执行器在别的线程上，不受影响） | 是（`block_on` 循环里 `self.run()`） |
| 同线程**本地队列**是否推进 | **否**（`!Send` 任务迁移不了） | `TrBlockOn`：否；`scope.block_on`：**只驱动它自己那条** | 是（同一份运行时） |
| 在 `LocalSet` 的驱动栈内调用 | **panic** | 无此概念 | 无此概念 |
| `current_thread` 运行时内调用 | **panic** | 无此概念 | 无此概念 |

`[源码事实]` tokio 1.53.2 `src/runtime/scheduler/multi_thread/worker.rs:427-436` 的注释原文：

> This probably means we are on the current_thread runtime or **in a LocalSet, where it is
> _not_ okay to block**.

`[实测]`（`cargo run --bin tokio_localset_block_in_place`，`ls_misuse_probe/`）：

```text
[tokio] 在 scope.run_until(..) 内部调 TrBlockOn::block_on      → can call blocking only when running on the multi-threaded runtime
[tokio] 在 scope.run_until(..) 内部调 TrLocalScope::block_on   → can call blocking only when running on the multi-threaded runtime
```

而 abs_art 的两个 tokio `block_on` 实现都以 `block_in_place` 开头，因此都被这条拦住。
**注意这条 panic 与「已在运行时上下文内即可用」的旧文档直接冲突**：`LocalSet` 内恰恰就是
「已处于上下文内」。所以旧文档不仅是措辞问题，它把一条 panic 路径描述成了可用路径。

`[实测]`（`cargo run --bin sched_nesting`）：

```text
[tokio/multi_thread(worker=1)] 旁观任务增量：TrBlockOn::block_on = 5865；TrLocalScope::block_on = 6135
[tokio/current_thread] TrBlockOn::block_on → can call blocking only when running on the multi-threaded runtime
[smol] TrBlockOn::block_on 期间，外层 LocalExecutor 上旁观任务的增量 = 0
[smol] scope.block_on（驱动别的队列）期间，外层 LocalExecutor 旁观任务增量 = 0
[smol] scope.block_on（驱动自己的队列）期间，同队列旁观任务增量 = 50200
[compio] TrBlockOn::block_on 期间，同运行时旁观任务增量 = 92
```

`[实测]`（`cargo run --bin tokio_migrate_smol_global`）：tokio 的「不影响调度」是**把任务挪走**，
不是「在本线程上轮转」：

```text
[tokio/worker 内] 外层任务线程 = ThreadId(2)；block_on 前旁观线程 = [ThreadId(2)]；
                  block_on 期间旁观线程 = [ThreadId(3)]；旁观增量 = 5460
[smol] TrBlockOn::block_on 期间，**全局执行器**旁观任务增量 = 46098
```

**结论**：同一个 `scope.block_on` 在三个后端上分别是「panic / 只驱动自己那条 / 顺带驱动整个
运行时」。把它留在 trait 上等于给出一个假承诺，因此删除；异步驱动保留为
`run_until`，阻塞入口只留在运行时值的 `TrBlockOn::block_on` 上（它**不**驱动本地队列）。
需要「阻塞 + 驱动」时写组合：`rt.block_on(scope.run_until(f))`。

---

## 2. 为什么把队列放进 `thread_local!`（因果链）

`[实测]`（`cargo run --bin ls_misuse_probe`，即 `ls_misuse_probe/src/main.rs`）
上一版形状下，「同一个运行时值上取两次 `local_scope()`」是**两条队列**：

```text
[tokio] 同值取两次：A 投递 / B 驱动 = Err(Elapsed(()))     ← 静默不推进
[tokio] 同值取两次：A 投递 / A 驱动 = Ok(Ok(42))
[tokio] clone：c 投递 / a 驱动 = Ok(Ok(7))
[tokio] drop 作用域值之后，detach 的本地任务是否推进 = false
[smol] A 投递：只驱动 B 之后 = false；驱动 A 之后 = true（取值 Ok(42)）
[compio] A 投递 / B 驱动 = 42（任务推进 = true）            ← 同一个错写法在 compio 上是对的
```

两个直接后果：

1. **「线程独占」这个措辞与语义不符**：它描述的是 `!Send`（这个值不能跨线程搬），
   而不是「每线程一条」。下游按直觉写「反正在同一条线程，再取一个 scope 来驱动」，
   在 tokio / smol 上就得到静默挂起，在 compio 上却完全正常——**最坏的失败模式**。
2. **生命周期错误静默**：作用域值一 drop，队列随之消失，`detach()` 出去的任务一起没了
   （实测 `false`）。

因此把队列的形状统一到 compio 那一侧：**队列属于线程，`LocalScope` 只是它的别名**。

---

## 3. 改了什么

| 位置 | 改动 |
| --- | --- |
| `abs_art/src/runtime.rs` | `TrLocalScope` 删除 `block_on`；`run_until` 文档给出**三后端语义表**（谁 poll、前提是什么）；类型文档改写为「线程本地对象的别名」；模块文档同步（并记下 tokio `LocalSet` 内的 panic 与源码出处）；`TrBlockOn` 文档改为「只等待、不驱动队列」 |
| `abs_art-tokio` | `thread_local! { static LOCAL_QUEUE_: Rc<LocalSet> }`；`LocalScope{handle_, local_}` 中 `local_` 来自 TLS（`handle_` 只服务于 `mock-clock` 与 escape hatch）；`local_scope()` 幂等；删除 `block_on`；`lib.rs` 增加 `#[cfg(any(feature = "local_scope", test))] extern crate std;`；单测全部改为**独立线程**运行，并新增 `separate_calls_share_the_thread_local_queue` |
| `abs_art-smol` | 同样进 `thread_local!`（`Rc<LocalExecutor<'static>>`）；删除 `block_on`；`LocalScope::with_executor()` 改名 `for_current_thread_()`；旧单测 `separate_scopes_have_separate_queues` **反转为** `separate_calls_share_the_thread_local_queue`；新增 `nested_run_until_drives_the_same_queue`（实测 smol 的 `LocalExecutor::run` 可重入） |
| `abs_art-compio` | 只删 `TrLocalScope::block_on` 实现与相关文档；类型/`local_scope()` 文档写明它是**对齐基准**（并如实记录「同一线程多份 `with_runtime` 实例各有一条队列」这一 compio 自身性质） |
| `abs_art-bridge` / `abs_art-demo` / `abs_art-smoke` | 所有 `scope.block_on(..)` 改为 `rt.block_on(scope.run_until(..))` 组合；D 用例改名为「阻塞组合」，判定标准不变；示例新增 `BLOCK_ON` 声明（阻塞发生在运行时值上，就得写下来） |
| `README.md` | 收敛叙述改为四轮；分工表、分层图、三后端对照表、契约矩阵同步；补「作用域上没有阻塞入口」的说明 |

约定（本轮确立）：

- `TrLocalScope` 只有 `spawn_local` + `run_until`；
- `Clone` 是**别名**，不是新建；
- 同一线程上多次 `local_scope()` 是**同一条**队列；
- 「同一线程多条 `LocalSet`」不支持（只为兼容 tokio 而保留其 runtime，不保留其多队列用法）；
- 阻塞入口只在 `TrBlockOn` 上，且不驱动本地队列。

---

## 4. 代价与后果（不隐瞒）

1. **队列寿命 = 线程寿命**：不能提前回收，线程退出时残留任务被丢弃。这是把「值亡则队列亡」
   换成「线程亡则队列亡」。
2. **测试隔离要靠线程**：libtest 会复用线程（`--test-threads=1` 时更是同一线程跑完所有用例），
   同一线程内的用例会共用同一条队列。本轮的选择是不提供 `reset()` / `with_scope()`，
   而是在受影响的三后端单测里**每个用例起一条独立线程**（`in_fresh_thread_`）。
3. **失去了「同一线程多条队列」这个能力**：tokio 允许、但本家族显式不支持；需要隔离语义的
   场景改为「换线程」。
4. **`block_on_advancing` 仍是一次阻塞驱动**：它保留为固有方法（`mock-clock` feature），
   名字里的 `block_on` 只描述它自己做的那次阻塞，文档写明它不属于 `TrLocalScope`。
   tokio 侧它仍走 `block_in_place`，因此**不得**在 `LocalSet` 的驱动栈内调用。
5. **公开 API 变更（破坏性）**：`TrLocalScope::block_on` 删除。已按 `AGENTS.md` 第 1 条
   先讨论后实施。

---

## 5. 验证（全部实跑）

| 项 | 命令 | 结果 |
| --- | --- | --- |
| 全 workspace | `cargo test --workspace` | **EXIT=0**（各 crate 单测 / doctest / 契约矩阵全绿，`abs_art-smoke` 13 + 24） |
| tokio 后端 | `cargo test -p abs_art-tokio --all-features` | 25 passed + 9 doctests + 2 compile_fail |
| smol 后端 | `cargo test -p abs_art-smol --all-features` | 36 passed + 9 doctests + 2 compile_fail |
| compio 后端 | `cargo test -p abs_art-compio --all-features` | 31 passed + 16 doctests |
| demo 两组 | `cargo test -p abs_art-demo [--no-default-features --features demo-tokio]` | 全绿（两组） |
| bridge 三后端同开 | `cargo check -p abs_art-bridge --all-targets --no-default-features --features default-backend-compio,backend-tokio,backend-smol` | 通过 |

---

## 6. 留作后续

- `TrBlockOn::block_on` 在 tokio 上的两条 panic 路径（`current_thread`、`LocalSet` 内）
  已写进文档，但**没有编译期/测试期的负向断言**；可考虑补 `#[should_panic]` 用例把它钉住。
- `smol` 的 `TrBlockOn::block_on` 会静默饿死同线程的本地队列（实测增量 0）。
  这是「阻塞入口不在作用域上」的又一条理由，但同样只在文档里。
- 三后端的 `LocalScope` 都没有 `Send` 的**负向**编译断言（stable 写不出否定约束），
  目前靠 `Rc` 的传染性保证。
