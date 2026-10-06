# 两个 API 提案的裁决：`current() -> &'static Runtime` 与 `local_clone(&self)`

日期：2026-10-06 10:43
分支：`feat/abs_art-runtime`（值化改造已落地，见 `runtime-value-20261006-1022.md`）
性质：**可行性实测 + 设计裁决**（本轮不改公开 API，只加探针）
探针：`abs_art_runtime_probe/p6_static_runtime/`（隔离 workspace，5 个成员 + 4 份证据日志）

---

## 0. 提案与判定

| 提案 | 判定 | 一句话理由 |
| --- | --- | --- |
| P1：`Runtime::current() -> &'static Runtime` | ❌ **不可行**（对带本地队列的运行时） | `static` 要求 `Sync`，而本地队列是 `Rc<LocalSet>` |
| P2：`local_clone(&self) -> Runtime`（不实现 `Clone`，因为有的运行时不能 clone） | ⚠️ **方向对，但一个名字盖了两种语义** | 「共享同一条队列」与「同运行时 + 新队列」必须分开命名 |

---

## 1. P1 为什么不可行：三层，前两层是类型系统直接拒绝

### 1.1 第一层：带本地队列的运行时值**放不进 `static`**

`[实测]` `cargo check -p p6_static_fail`（tokio 默认 features，`local_scope` 开启）：

```text
error[E0277]: `Rc<tokio::task::local::LocalSet>` cannot be shared between threads safely
  --> p6_static_fail/src/lib.rs:12:16
   |
12 | pub static RT: OnceLock<Runtime<{ FULL }>> = OnceLock::new();
   |                ^^^^^^^^^^^^^^^^^^^^^^^^^^^ `Rc<tokio::task::local::LocalSet>`
   |                                            cannot be shared between threads safely
   = help: within `Runtime<31>`, the trait `Sync` is not implemented for `Rc<LocalSet>`
   = note: shared static variables must have a type that implements `Sync`
```

三个后端的本地队列分别是 `Rc<LocalSet>`（tokio）、`Rc<LocalExecutor>`（smol）、
`Rc` 句柄簇（compio）——**都不是 `Sync`**，所以「持有本地队列的运行时值」永远不可能有
`&'static`。这不是实现技巧问题，是本地队列的**线程绑定**本质决定的。

`[实测]` 对照 `cargo check -p p6_static_ok`：把 `local_scope` 关掉（值只剩
`tokio::runtime::Handle`，`Send + Sync`），**同一个 `static` 形状就编译通过**。
所以能做成全局单例的只有「**不含本地队列**」的运行时——代价是它没有 `TrLocalScope`
（没有 `spawn_local` / `run_until`）。

### 1.2 第二层：按 `CAPS` 实例化的静态项不存在

`[实测]` `cargo check -p p6_generic_static_fail`：

```text
error[E0401]: can't use generic parameters from outer item
11 | pub fn current_static<const CAPS: usize>() -> &'static Runtime<CAPS> {
12 |     static RT: OnceLock<Runtime<CAPS>> = OnceLock::new();
```

Rust 没有「按 const 泛型实例化的静态项」（no polymorphic statics）。于是
`Runtime::<C>::current() -> &'static Runtime<C>` 对**任意** `C` 都不成立；
只能给某一个固定实例（`FULL`）写死一个 `static`。声明子集能力时要么拿不到
`'static`，要么再 `retag` 出一个 owned 值——`'static` 的便利就没了。

### 1.3 第三层：绕法存在，但代价可测

`thread_local!` + `Box::leak` 能得到 `&'static`（`p6_tls_leak`）。`[实测]` 两条：

| 情形 | 未完成的本地任务（持有哨兵）何时被析构 |
| --- | --- |
| **调用方持有**的值：`let value = current(); …; drop(value)` | **`drop(value)` 当场析构**（哨兵计数 1） |
| `thread_local! + Box::leak` 的 `'static` 值 | **永不析构**（哨兵计数 0） |

即：值化模型下「队列随值存活、值亡则未完成任务随之回收」这条性质，
在 leak 出 `'static` 之后变成了「队列与里面的任务一起永生」——挂起的任务所持有的
缓冲、句柄、锁统统不再释放（`evidence/04_pending_task_drop_timing.log`）。

此外 leak 方案天然是**每线程一份**的隐式全局状态：
- 同进程多个运行时 / 嵌套运行时会互相串（这轮值化正是为了消除它）；
- 测试并行跑时共用同一份，互相干扰；
- 假运行时（DI / 确定性验收）无法注入。

### 1.4 P1 想解决的问题，`'static` 并不是唯一解（也不是最好的解）

要解决的其实是「future 借用了运行时值 → 返回值生命周期绑在调用方栈帧上」
（`abs_art-smoke` 里 `FnOnce(&R) -> F` 表达不了「返回值与入参借用同期」，见
`runtime-value-20261006-1022.md` §7.4）。三种解法：

| 解法 | 是否引入全局状态 | 对 `!Send`/`!Sync` 的值是否成立 | 代价 |
| --- | --- | --- | --- |
| 借 `&'static` | **是** | 否（带队列的值根本进不了 static） | §1.1~1.3 |
| **拥有**（`Clone` / `Rc<Runtime>`） | 否 | **是**（单线程 `Rc` 即可） | 一次引用计数 |
| `Arc<Runtime>` | 否 | 否（需要 `Sync`） | 引用计数 + `Sync` 约束 |

**结论：把运行时**拥有**在 future 里（clone 或 `Rc`）就能拿到 `'static` 效果，
且不需要任何全局状态。** 这正是 P2 想做的事。

若确实需要「全进程一份」的便利，正确形状是**由应用侧持有**：
`Rc<Runtime>`（单线程）/（对不含本地队列的值）`OnceLock<Runtime>`，
或者应用自己 `Box::leak`——把「我接受它永不析构」这个决定留在应用里，
而不是写进库的 API。

---

## 2. P2 方向对，但 `local_clone` 这个名字盖了两种语义

`[实测]`（`p6_clone_semantics`，三条全绿）：

| 语义 | 今天怎么拿到 | 实测行为 |
| --- | --- | --- |
| (i) 共享运行时 + **共享同一条**本地队列 | `value.clone()` | 克隆体投递的任务，**原值驱动就能跑完**（取回 7） |
| (ii) 共享运行时 + **新**一条本地队列 | `Runtime::with_handle(value.handle().clone())` | 新队列上的任务**不会**被原值的驱动推进（`Cell` 保持 false） |

两种都「实质上是同一个运行时」，但对本地任务的可达性完全相反。
(ii) 在 tokio 上尤其危险：`LocalSet` 绑定**创建它的线程**，「克隆一个到别的线程去驱动」
是错的；任务会静默留在没人推的队列里。

因此：

- **不要把 `Clone` 放进抽象 trait**（你的直觉对）：现状就是 `Clone` 只作为各后端类型
  自己的 inherent impl 存在；库要「拥有」就写 `R: Clone`，不需要就收 `&R`。
- **`local_clone` 这个名字要拆成两个自解释的名字**，例如
  `share(&self) -> Self`（(i)）与 `with_local_queue(&self) -> Self` / `fork_local(&self)`（(ii)）。
  `local_clone` 字面像 (ii)（「local」），但「实质上是同一个 Runtime」的直觉指向 (i)，
  使用者极可能踩到 (ii) 的静默不驱动。
- `local_clone` 相对今天**新增的能力只是命名**，不是能力——三个后端现在都能 `Clone`
  （`p6_clone_semantics` 编译期断言过）。它真正的价值是：给「将来某个不能 clone 的后端」
  留一个**可选**的 inherent 方法位，而不是把它变成 trait 要求。

---

## 3. 顺带发现（已修）

- `abs_art-tokio`：关掉 `local_scope` 时 `use alloc::rc::Rc;` 变成未使用告警。已按
  feature 门控（`#[cfg(feature = "local_scope")]`），并复跑：默认 features 与
  `--no-default-features --features block_on,delay,spawn_send,spawn_blocking` 两种配置都干净。
- **const 泛型值类型的构造坑**：`Runtime::current()` / `Runtime::with_handle(..)` 这类
  关联函数写在**表达式位置**时会 `E0284`（缺失的 `CAPS` 无法从使用处推断，默认值
  不参与推断）。目前只有 crate 级自由函数（`abs_art_tokio::current()`）免疫。
  低成本缓解：给其余构造函数也配自由函数（`abs_art_tokio::with_handle(h) -> Runtime<FULL>`），
  或让调用点写类型标注。**留作待裁决项**。
