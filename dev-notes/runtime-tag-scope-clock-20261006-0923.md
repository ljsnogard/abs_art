# 三项构想（第二轮实测）：`Runtime<CAPS, B>`、thread-local `LocalScope`、`TrClock` 作为 Cap

> **后记（2026-10-06 14:20）：本文 §2 的探针结论已成为落地形状。** 当时判定 thread-local 的
> `LocalScope`「部分可行」并列了六条代价；本日裁决把它采纳为**强制语义**（队列属于线程、
> 作用域只是别名），并连带删除 `TrLocalScope::block_on`。六条代价的处置——测试隔离改为
> 「用例各自起独立线程」、不提供 `reset()` / `with_scope()`——与实测见
> `local-scope-thread-local-20261006-1420.md`。

日期：2026-10-06 09:23（想法 1 部分于 09:45 补完实测）
性质：**可行性实测**（本轮**不动生产代码**；全部结论来自隔离的探针 workspace）
来源：承接 `bridge-scope-clock-20261005-1615.md`（下称「前文」）。前文 §1.4 用
`impl a::TrDelay for a::Runtime<1>` 一个**单一反例**就写下「`abs_art::Runtime<B, CAPS>`
可行，但调用点仍要写 B，等于把『每后端一个类型』换个写法、并没有省掉名字」——
**前半句（可行）没实测过，后半句（省不掉名字）本轮实测成立，但理由与前文不同**。
本轮把 tag 形态实测到底，并把前文 §2（thread-local `LocalScope`）与 §3（`TrClock`）
从构想推进到可运行证据。

探针目录（均不与仓库其它部分共享构建）：

| 目录 | 内容 |
| --- | --- |
| `abs_art_runtime_probe/p1_orphan/` | 想法 1：孤儿规则 / 固有 impl / 默认 B / 投影形态 / bridge 对照 |
| `abs_art_runtime_probe/p2_tagged/` | 想法 1 厚形态在**真实 `abs_art` trait** 上的端到端 |
| `abs_art_runtime_probe/p2_tls/` | 想法 2：thread-local 队列 + 自由函数驱动 |
| `abs_art_runtime_probe/p3_clock/` | 想法 3：`TrClock` 结构约束与能力位扩张 |

## 0. 结论速览

| 项 | 判定 | 一句话理由 |
| --- | --- | --- |
| 1 · 薄形态（后端直接为 `Runtime<CAPS, Tag>` 实现能力 trait） | ❌ **不可行** | `E0117`：外来泛型类型里的本地类型**不算**本地类型 |
| 1 · 厚形态（`Backend` 族 trait + blanket impl） | ✅ 可行 | 真实 trait 端到端 `EXIT=0`（§1.4） |
| 1 · **投影形态**（`HasRuntime` GAT + blanket impl） | ✅ 可行，**代价最低** | 真实 trait 端到端 `EXIT=0`（§1.5）；可做成**纯新增 crate** |
| 1 · 「默认 `B` 由 feature 给」 | ❌ **抽象层给不出** | 不可能三角：`E0117` / 循环依赖 / 必须把后端并进抽象层（§1.6） |
| 1 · 「于是代码里只有一个 runtime」 | ⚠️ 只在 bridge/二进制层成立 | **库侧仍是泛型，参数个数不减**（§1.7） |
| 1 · 「bridge 多后端同存」这个目标 | ✅ 但**不需要想法 1** | 旧形态用 `cfg` 门控裸名 + 按名导出即可（`EXIT=0`，§1.8） |
| 1 · **改由 `abs_art-bridge` 承载「唯一默认 runtime」** | ✅ **可行**（实测） | 库不再被 feature 传染（dev-dep feature 不泄漏）；`abs_art` 零改动；见 §6 |
| 2 · thread-local `LocalScope` | ✅ **部分可行**（tokio/smol 均绿） | 自由函数 + TLS 成立、嵌套成立；代价是**驱动契约变严且静默失败**、**测试隔离要自己还**（§2.2） |
| 3 · `TrClock` 作为 Cap（结构约束） | ✅ 可行 | 四种真实时刻类型全部满足约束（§3.1） |
| 3 · 新开 `CLOCK = 1<<5` 能力位 | ✅ 可行，需扩表 | 表从 80 个数字涨到 192 个（宏生成可解）；`FULL` 31→63 是公开行为变更（§3.2） |
| 3 · 「同源」由类型系统保证 | ❌ 做不到（只能钉名字） | `TrClock<Instant = T::Instant>` 可钉类型，钉不住时间基准（§3.3） |

---

## 1. 想法 1：`abs_art::Runtime<CAPS, B>`

### 1.1 三种形态必须分开说

原始构想的关键句是「`B` 是后端本地的标记类型，后端 `impl Backend for B` 合法——trait
外来、类型本地」。这句话本身没错，但它对应**三种完全不同的实现形态**，可行性/代价差别很大：

```rust
// 形态①「薄」：后端直接为抽象层的 Runtime 实现能力 trait
// (abs_art-tokio)
pub struct TokioTag;
impl<const CAPS: usize> abs_art::TrDelay for abs_art::Runtime<CAPS, TokioTag> { .. }   // ❌ E0117

// 形态②「厚」：抽象层定义 Backend 族 trait + blanket impl，后端只实现 Backend
// (abs_art)
pub trait BackendDelay { type Delay; fn delay(d: Duration) -> Self::Delay; }
impl<const CAPS: usize, B: BackendDelay> TrDelay for Runtime<CAPS, B>
where [(); CAPS]: HasDelay
{ type Delay = B::Delay; fn delay(d: Duration) -> Self::Delay { B::delay(d) } }
// (abs_art-tokio)
impl BackendDelay for TokioTag { type Delay = tokio::time::Sleep; .. }                  // ✅

// 形态③「投影」：抽象层只要一个 GAT，能力 impl 仍留在后端的本地类型上
// (abs_art 或新 crate)
pub trait HasRuntime { type Rt<const CAPS: usize>; }
impl<const CAPS: usize, B> TrDelay for Runtime<CAPS, B>
where B: HasRuntime, <B as HasRuntime>::Rt<CAPS>: TrDelay, [(); CAPS]: HasDelay
{ type Delay = <<B as HasRuntime>::Rt<CAPS> as TrDelay>::Delay; .. }                     // ✅
// (abs_art-tokio)
impl HasRuntime for TokioTag { type Rt<const CAPS: usize> = crate::Runtime<CAPS>; }      // ✅
```

### 1.2 形态①不可行：`E0117`（实测，决定性）

`[实测]` `cargo check -p app`（`p1_orphan`，`ab/src/lib.rs:16`，见 E1）：

```text
error[E0117]: only traits defined in the current crate can be implemented for types
              defined outside of the crate
  --> ab/src/lib.rs:16:1
   |
16 |   impl<const CAPS: usize> TrDelay for aa::Runtime<CAPS, TokioTag>
   |   ^                                   --------------------------- `aa::Runtime` is not
   |                                                                  defined in the current crate
   = note: impl doesn't have any local type before any uncovered type parameters
```

`TokioTag` 确实是本 crate 定义的本地类型、也确实在 `Runtime` 的实参位置上，rustc 仍判定
「没有任何本地类型」。**孤儿规则不递归进外来类型的类型实参去找本地类型**（只有
`#[fundamental]` 包装如 `&T` / `Box<T>` 才透明，用户类型不能标 `fundamental`）。

`[实测]` 三个边界刻划（`ab2_*`，见 E1）：

| 写法 | 结果 |
| --- | --- |
| `impl TrDelay for aa2::Rt<TokioTag>`（外来类型，本地类型是它**唯一**实参） | `E0117` |
| `impl<const C: usize> TrDelay for aa2::RtC<TokioTag, C>`（本地实参在前、`const` 在后） | `E0117` |
| `impl TrDelay2<TokioTag> for aa2::ForeignSelf`（本地类型在 **trait 的实参**位置） | `EXIT=0` |

即：本地类型只有在 **impl 表头里自己作为类型出现**时才作数（Self 位置、或 trait 的泛型
实参位置）。这也顺带解释了为什么今天必须「每个后端一个 `Runtime`」——那是**唯一**合法的位置。

### 1.3 形态②单独说：固有 impl 会被禁掉

`[实测]` `cargo check -p fail_inherent`（E2）：

```text
error[E0116]: cannot define inherent `impl` for a type outside of the crate where the type is defined
```

今天三个后端各有 6 个固有 impl（`current` / `tag` / `block_on` / `local_scope` 等，共 18 个）。
形态②下这些必须搬到抽象层（`impl<const CAPS: usize, B: BackendXxx> Runtime<CAPS, B>`），
后端若还想留自家便利方法只能用**自由函数**或**本地扩展 trait**——`[实测]` 两条都可用
（`p1_orphan/ab_ext`，见 E10），但书写形状要改。

### 1.4 形态②可行：真实 trait 端到端（`p2_tagged`）

`[实测]` `abs_art_runtime_probe/p2_tagged/`（抽象层替身 `art_rt` + 真 tokio 后端
`art_rt_tokio` + `app` + `bridge` + `badcaps`），关键结果：

| 项 | 命令 | 结果 |
| --- | --- | --- |
| 六条 blanket impl（含 GAT、`TrTime: TrDelay` 超 trait、能力位门控、tag 无关的 `Send + Sync`） | `cargo test -p art_rt` | 6 passed，`EXIT=0`（E12） |
| 端到端（tag / `block_on` / `delay` / `interval` / `timeout`×2 / `spawn` / `local_scope` / `spawn_blocking` / `about`） | `cargo run -p app` | `P2-APP-OK`，`EXIT=0`（E13） |
| bridge 双后端同存 | `cargo check -p bridge --features backend-compio` | `EXIT=0`（E14） |
| 能力位门控仍生效 | `cargo check -p badcaps` | `E0277`（`art_rt::Runtime<1, Tokio>` 不实现 `TrSpawnSend`），`EXIT=101`（E15） |

`[实测]` 形态②下**下游写不出**与 blanket impl 竞争的直接 impl：`cargo check -p conflict`
先撞 `E0117`（E5）。所以不存在前文担心的 `E0119` 叠加冲突（竞争者根本写不出来）；
代价是扩展性收窄——后端只能走 `Backend` 族。

### 1.5 形态③可行且代价最低：投影（本轮新增）

`[实测]` 最小探针 `p1_orphan/{cc,cc_ab,app_cc}`（含**带 `where T: 'static` 的 GAT 投影**）
`cargo run -p app_cc` → `EXIT=0`（E16）。

`[实测]` 更关键的一步：把同一形状对着**真实 `abs_art` trait** 跑通
（`p1_orphan/{proj_rt,proj_be,proj_app}`，覆盖 `TrBlockOn`（无 `'static`）/ `TrDelay` /
`TrTime` / `TrSpawnSend` / `TrSpawnBlocking` / `TrAsyncRuntime`，全部带
`[(); CAPS]: HasXxx` 门控）：

```console
$ cargo run -p proj_app
P1 投影形态（真实 abs_art trait）：EXIT=0
```

**投影形态的代价表（相对今天）**：

| | 形态②厚（`Backend` 族） | 形态③投影 |
| --- | --- | --- |
| `abs_art` 新增 | `Runtime<CAPS, B>` + **6~7 个** `Backend*` trait（含 GAT/关联类型） | `Runtime<CAPS, B>` + **1 个** `HasRuntime`（1 个 GAT）+ 6 条 blanket impl |
| 后端改动 | 18 个能力 impl 全部改写；18 个固有 impl 搬进 `abs_art` | 18 个能力 impl **原样不动**；18 个固有 impl **留在后端**；只加 tag + 1 个 `HasRuntime` impl + 别名 |
| 库侧约束写法 | `B: BackendDelay`（干净，一条） | `B: HasRuntime, B::Rt<CAPS>: TrDelay`（啰嗦，两条且带投影） |
| 能否不动 `abs_art` | 不能（blanket impl 必须在 `abs_art` 内） | **能**：整套可放进一个**新增 crate**（`Runtime` 与 `HasRuntime` 都属于新 crate，blanket impl 是「外来 trait + 本地类型」，合法） |
| 后端可扩展性 | 收窄（只能走 `Backend` 族） | 不变（后端仍拥有自己的类型与固有方法） |

`[源码事实]` 今天三个后端共有 **18 个 `impl … for Runtime<CAPS>`** 与 **18 个固有 impl**，
后端源码合计约 3200 行。**投影形态几乎不动这 3200 行**，这是它最实在的优势。

> 投影形态还可以更省：`HasRuntime` 连 GAT 都不需要——`pub type TaggedRuntime<const CAPS: usize>
> = Runtime<CAPS, abs_art_tokio::Runtime<CAPS>>;` 直接可用，代价是**库侧无法泛型**（`B`
> 必须已经是「带 CAPS 的运行时类型」，无法用类型参数表达）。要库侧可用，GAT 是必需的。

### 1.6 「默认 `B` 由 feature 给」：抽象层给不出（不可能三角）

原始构想里「通过 feature 让 `B` 有不同默认值」这一步，**只能在同时看见抽象层与后端的
crate 里做**（bridge，或最终二进制自己的 type alias）。三条路都实测过：

| 尝试 | 结果 |
| --- | --- |
| `abs_art` 自己定义 tag，后端 `impl Backend for abs_art::TokioTag` | `E0117`（E3：trait 与类型都属于抽象层） |
| `abs_art` 开 `feature = "backend-tokio"` 依赖 `abs_art-tokio` | **循环依赖**：`error: cyclic package dependency: package cyc_a depends on itself`（E8） |
| `abs_art` 只留默认 `B = ()` | `E0277: the trait bound Runtime<3>: TrDelay is not satisfied`（E6） |

只有一条形状能让**单参数** `Runtime<CAPS>` 真正可用：**把 tag 与 impl 都并进抽象层**，
用 `#[cfg(feature = ...)]` 选默认 tag（`p1_orphan/one/`，`EXIT=0`，E9）。但那意味着
`abs_art` 直接依赖 `tokio` / `compio` / `smol`——家族今天「抽象层零依赖、后端各自成 crate」
的结构就没了。

> **不可能三角**：以下三条不能同时成立——
> (i) 抽象层拥有 trait 与 `Runtime` 类型；(ii) 后端 crate 拥有 tag 类型并写 impl；
> (iii) 抽象层给出**可用的**默认 `B`。
> 因为 (iii)+(i)+(ii) 需要 `abs_art → backend` 依赖，而 (ii) 需要 `backend → abs_art`。

**推论**：`Runtime<CAPS>`（单参数）在 `abs_art` 里永远是 `E0277`。能用的写法只有两种：
后端别名 `abs_art_tokio::Runtime<CAPS>`，或 bridge 别名
`abs_art_bridge::Runtime<CAPS> = abs_art::Runtime<CAPS, 被选中的 Tag>`。

### 1.7 收益核算：什么真的变了、什么没变

**变好了**：

1. 抽象层第一次有了**可命名的「运行时」符号**。今天这个符号只存在于各后端 crate，抽象层
   只有一个 0 泛型的 `enum RuntimeTag`（前文 `runner_concrete_fail` 的第一障碍正是
   「库根本命名不出 `Runtime<CAPS>`」）。
2. 能力（`TrDelay` / `TrTime` / 未来的 `TrClock`）可以挂在**运行时类型**上，而不是挂在
   「本地队列的值」`LocalScope` 上——这正是前文 §3.4 指出的建模错位。
3. 后端从「一个运行时类型」变成「一个 tag + 一个别名」，多后端在同一二进制里的关系从
   「互不相干的类型」变成「同一类型的不同实例」（形态③下这条收益打折：后端类型仍在）。

**没有变**：

1. **库侧的泛型参数个数不减。** 业务库要写 `abs_art::Runtime<CAPS, B>` 就必须自己引入 `B`
   （`B: BackendXxx`，或形态③的 `B: HasRuntime, B::Rt<CAPS>: TrXxx`），与今天的
   `Rt: TrSpawnSend + TrDelay` 是**同一个形状、同一个数量**。除非库依赖 bridge——那就把
   「选后端」的传染带回来了（前文 §1 的问题）。
2. **`B` 这个名字没有消失**，只是从「每个后端一个 `Runtime`」变成「每个后端一个 tag
   （+ 形态③下仍保留自己的 `Runtime`）+ 一个别名」。调用点看起来只有一个 runtime，靠的是
   别名，而别名今天就有（bridge 的重导出）。
3. **bridge 的「多后端同存」不需要它**（§1.8）。

净效果：想法 1 是一条**架构归属的重构**（把「运行时」这个概念搬到抽象层），**不是一次能力
剪裁**——它不会让任何现有代码少写一个参数。

### 1.8 与「旧形态 + bridge 按名导出」的对照（这条最影响裁决）

前文 §1.5 的构想（单后端保留裸名、多后端按名导出）**不需要 tag 也能做**。`[实测]`
`p1_orphan/old_bridge`（两个后端各自定义本地 `Runtime`，bridge 用 `cfg` 门控裸名 +
按名导出），两个 backend feature 同时开启：

```console
$ cargo run -p app_old_bridge
P1 old_bridge：旧形态下「多后端同存 + 裸名」也可行，EXIT=0
```

tag 形态的同一场景（`p1_orphan/bridge2`）也是 `EXIT=0`（E7）。**两者都行，所以「让 bridge
支持多后端」不是想法 1 的独有收益。** 实际差别：

| | 旧形态（每后端一个 `Runtime`） | tag 形态 |
| --- | --- | --- |
| 裸名 | `cfg` 门控重导出，只有一个 | `cfg` 门控类型别名，只有一个 |
| 具名导出 | `pub use abs_art_tokio::Runtime as TokioRuntime` | `pub type TokioRuntime<..> = abs_art::Runtime<.., Tokio>` |
| `LocalScope` | 同样按名导出即可 | **仍需按名导出/子模块**（tag 没有统一 `LocalScope`） |
| 底层类型 | 三个互不相干的 ZST | 一个类型的三个实例 |
| 抽象层改动 | 零 | 新增 `Runtime<CAPS, B>` +（②）`Backend` 族 /（③）`HasRuntime` |

### 1.9 待裁决（想法 1）

1. **是否值得做**：§1.7 的「没有变」三条说明它不解决「库少写参数」，§1.8 说明它不解决
   「bridge 多后端」。若目标只是这两条，建议**不做**；若目标是「让能力有地方挂」（前文
   §3.4 的建模错位）或「让抽象层能命名运行时」，再谈落地。
2. **若做，优先形态③（投影）**：迁移成本低一个数量级（后端 3200 行几乎不动、`abs_art`
   公开面只加 1 个 trait），且可以做成**纯新增 crate**（`abs_art-runtime` 之类）、`abs_art`
   本体零改动。代价是库侧约束写法啰嗦（`B: HasRuntime, B::Rt<CAPS>: TrXxx`）。
3. **若更看重库侧约束干净**，才选形态②，但要接受：`abs_art` 新增一族公开 trait（须按
   `AGENTS.md` 第 1 条先行讨论）、三个后端 36 个 impl 全部改写。
4. 无论哪种：`Runtime<CAPS, B>` 两个参数都**不给默认值**（给 `CAPS` 默认而 `B` 无默认没有
   意义），后端必须提供 `pub type Runtime<const CAPS: usize = FULL> = …` 以保住调用点形状。
5. 想法 1 与想法 2 有耦合：若能力表最终放在「std 桥的自由函数」里（前文 §2.3 的候选 (b)），
   运行时符号更没必要；若能力表要挂在类型上，则想法 1 是它的承载体。

---

## 2. 想法 2：thread-local 的 `LocalScope`

探针：`abs_art_runtime_probe/p2_tls/`（`tls-bridge-tokio` / `tls-bridge-smol` +
`tls-probe` 用例矩阵；同一份脚本用 `tls_experiments!` 宏实例化到两个后端）。

### 2.1 探针形状

`thread_local!` 里持 `RefCell<Option<Rc<LocalSet>>>`（tokio）/
`Rc<LocalExecutor<'static>>`（smol），对外**只有自由函数**：

| 函数 | 作用 |
| --- | --- |
| `spawn_local(fut)` | 投递 `!Send` 任务到本线程队列（无队列则懒新建） |
| `run_until(fut)` | 驱动本线程队列直到 `fut` 完成（**不收 `&self`**） |
| `block_on(fut)` | 阻塞驱动（tokio 侧内部走 `Handle::current()`） |
| `with_scope()` / `ScopeGuard` | 显式开始/结束本线程队列的寿命 |
| `reset()` | 拆掉本线程队列（连同未完成任务） |
| `has_scope()` / `scope_count()` | 观测 |

一个必须的实现细节：`run_until` **先 clone 出 `Rc` 再 await**，不跨 await 持有 TLS 的
`RefCell` 借用（`tls-bridge-tokio/src/lib.rs:181-185`）——否则重入会先撞借用冲突。

### 2.2 判定：**部分可行**（探针已绿）

> **更正（同日本轮）**：我先前独立复跑时该探针是红的（`EXIT=124`、tokio/smol 均有挂起与
> FAILED）。经与探针作者核对，那是一个**中间版本**的用例脚手架 bug——(i) 用例体没有一律
> 套 `run_until`、(ii) `block_on` 少了 `block_in_place`。作者修好后两种模式都绿：
> `cargo test --workspace` → **tokio 19 passed / smol 17 passed / doctests 4+3 passed**，
> 串行与默认并行**均 `EXIT=0`**（`p2_tls/evidence/E-*.log`）。下面按修好后的版本重述。

`[实测]`（探针作者，`p2_tls/evidence/`）

| 编号 | 结论 | 原始观测 |
| --- | --- | --- |
| E1 | 自由函数可用（a/b）：无类型名、无值参数地投递 `Rc` 的 `!Send` 任务并取回 42 | tokio/smol 均 `ok` |
| E2 | **「谁驱动」= 显式 `run_until`/`block_on`**；只写 `rt.block_on(spawn_local(..).await)` 会**静默挂死**（不是 panic 提示） | `[h-tokio] 不套 run_until 是否超时 = true`；smol 侧句柄始终 Pending |
| E3 | `run_until(async {})` **不会 tick 队列**（外层 Ready 直接返回）——要「只推队列」得让外层先 Pending 一轮 | `d` 用例 |
| E4 | 同线程**嵌套** `run_until` 不 panic、不死锁 | `c_nested_run_until ... ok`（两后端） |
| E5 | 连续两次 `run_until` 复用同一条队列（第一次未驱动完的任务在第二次被取回） | 取回 7 |
| E6 | **测试隔离丢失（正式证据，确定性构造）**：同线程两段用例体顺序执行，第一段 `detach` 的任务被第二段跑掉 | `[contamination-tokio] 用例体1 之后 = 0，用例体2 之后 = 1，scope_count = 1`（smol 同形） |
| E6b | `reset()` 能恢复隔离 | `[d2] reset 后：scope_count = 2，计数器 = 0` |
| E7 | 跨线程使用**静默新建**队列（不 panic、不报错） | `[e-tokio] 任务线程 = ThreadId(14)，has_scope(先) = false，scope_count = 1` |
| E8 | 线程退出时残留任务随线程消失 | `[e3] 线程退出后 EXECUTED = 0` |
| E9 | `block_on` 的前提：运行时上下文（否则 `there is no reactor running ...`）+ **多线程**运行时（否则 `can call blocking only when running on the multi-threaded runtime`） | 逐字 panic 文本 |
| E10 | `run_until(..)` 的 future 因跨 await 持 `Rc<LocalSet>` 而是 **`!Send`**，`tokio::spawn` 直接 `E0277` | 「跨线程搬运驱动入口」在类型层被否掉 |

### 2.3 与探针 bug 无关的设计层结论

1. **「谁驱动」的答案在「每个后端一个 std 桥 crate」，不在抽象层**：`spawn_local` /
   `run_until` 必须住在持有 `thread_local!` 的 std crate 里，而 `abs_art` 是 `no_std` +
   零依赖。于是调用方要么依赖**某个后端专属桥**（库就被钉死在一个后端上），要么需要
   一个**统一的环境 crate**——后者又回到前文 §2.3 的 (a)/(b) 裁决，并新增两个问题：
   feature 传染（多后端同二进制）与「同一线程两个后端同时安装」的冲突语义。
2. **类型系统会丢一条保证**：值形态下 `&LocalSet` 是 `!Send`，跨线程 `spawn_local`
   **编译不过**；TLS + 自由函数形态下这只能靠运行期检查——探针自己的文档就写着
   「跨线程投递会 `debug_assert` 失败（debug panic，release UB）」
   （`tls-bridge-tokio/src/lib.rs:139-141`）。
3. **寿命从「值」变成「线程」**：不显式 `reset()`，队列寿命 = 线程寿命
   （`tls-bridge-tokio/src/lib.rs:30`）。
4. **懒新建是双刃**：无队列时 `spawn_local` 不再 panic（今天 tokio 要求必须处于
   `LocalSet` 上下文），而是静默新建一条；若那个线程永远不被驱动，任务**静默丢失**。
5. **测试隔离需要显式 `reset()` 或 `with_scope()`**：所以「使用者不再需要手动保存一个
   `LocalScope`」要打个折——手动保存变成了手动重置/手动进入，只是后者只在需要隔离时写。
6. **与想法 1 的关系**：若环境能力表由 std 桥的自由函数提供，库侧连类型参数都不需要，
   想法 1 的收益进一步下降；若能力表要挂在类型上，想法 1 才是它的承载体。

---

## 3. 想法 3：`TrClock` 作为 Cap

探针：`abs_art_runtime_probe/p3_clock/`（19 个 cargo 成员 + `evidence/` 25 条日志 + ICE 留档）
与我自己补的两个独立复验 `abs_art_runtime_probe/z_clock/`、`p1_orphan/z_caps6/`。

### 3.1 时刻类型：结构约束成立，但**必须有载体类型**

`[实测]` 我自己独立复验（`z_clock/clock_ok`，`cargo test` `EXIT=0`）：四种真实时刻类型
**全部满足**前文 §3.3 草案里的结构约束

```text
type Instant: Copy + Ord + Add<Duration, Output = Self::Instant>
            + Sub<Self::Instant, Output = Duration> + 'static;
```

| 时刻类型 | 出处 | 结果 |
| --- | --- | --- |
| `std::time::Instant` | compio、smol/async-io | 满足 |
| `tokio::time::Instant` | tokio（`test-util` 可暂停） | 满足 |
| `embedded_timers::instant::Instant64<1000>`（及 `Instant32`） | 本仓库 `embedded-timers` | 满足 |
| `embedded_timers::instant::TimespecInstant` | 同上 | 满足 |
| `std::time::SystemTime` | — | **不满足**（缺 `Sub<Self>`，p3 a5） |
| 借用型 `BorrowedInstant<'a>` | 探针自造 | **不满足**（E0477，缺 `'static`） |
| 裸 `u64` | — | **不满足**（无 `Add/Sub<Duration>`） |

约束非空：`z_clock/clock_fail`（`type Instant = f32`）→ `E0271`。

`[实测]` **`impl TrClock for std::time::Instant` 在下游 crate 一律不合法**（trait 与类型
都外来 → `E0117`，p3 g2）。所以 `TrClock` 不是「时刻类型的 trait」，而是**运行时的取时
能力**：由各后端对自己的本地 `Runtime<CAPS>` 实现，用 `type Instant = …` 给出时刻类型。
这一点必须写进文档，否则很容易被误用。

放宽实验（p3 a4）：去掉 `'static` 对四个候选**收益为 0**（建议保留）；再去掉 `Sub<Self>`
会失去 `deadline - now` / elapsed 的唯一原语（不建议）。

### 3.2 能力位：新开 `CLOCK = 1<<5` 优于复用 `DELAY`

- **(甲) 新位**：`impl<const CAPS: usize> TrClock for Runtime<CAPS> where [(); CAPS]: HasClock`
  在三后端都能编；代价是默认 `Runtime<31>` 不再自动具备时钟——这恰好符合本家族
  「想用就得写下来」的能力位设计意图。
- **(乙) 复用 `DELAY`**：默认 `Runtime<31>` 自动有时钟，但**并不省事**——一旦存在第 6 位
  （任何用途），`HasDelay` 的表照样要扩到 0..=63（p3 c4 实测 `Runtime<63>` 反而没有时钟
  能力），而且语义上把「能等」与「能读表」混为一谈。
- **扩表可行，代价明确**：`impl_has!` 表从 5×16 = **80** 个数字涨到 6×32 = **192** 个
  数字（`2^(n-1)`/张），手写 6 行、每行 103–109 字符。`[实测]` 我独立复验了**宏生成**版本
  （`p1_orphan/z_caps6`，`cargo test` `EXIT=0`，192 个 impl 全由 `macro_rules!` 展开，
  每个 trait 只需一行参数）；p3 修正后的 `caps63` 也 6 tests passed，并把手写表与宏表
  **逐元素**与独立算出的集合比对过。
- **`generic_const_exprs` 不可用**（p3 实测，重要）：nightly 正向能编但带
  `feature(generic_const_exprs) is not supported with the next-generation trait solver`
  的 warning（该 crate 退回旧 solver）；**负例直接让 rustc ICE**
  （`the compiler unexpectedly panicked`，留档 `p3_clock/evidence/ice/`），而且 rustdoc 会把
  ICE 记成 `compile fail ... ok` 的**假通过**；stable 则是 `E0554`。结论：稳定解法是
  `macro_rules!` 生成（或手写表），不要用 GCE。
- **公开面影响**：`caps.rs` 的 `bit_assignment_is_stable` 必然要改（`FULL` 31 → 63、
  新增 `assert_eq!(CLOCK, 32)`）；前五位数值不变。但下游写死的 `Runtime<31>` 的**语义**
  变了（从「全部能力」变成「除了时钟的全部能力」），属于公开行为变更，按 `AGENTS.md`
  第 1 条须先讨论。

### 3.3 「同源」：类型系统只能钉名字，钉不住时间基准

- `[实测]` 反例（p3 d1）：`fn f<T: TrTime, C: TrClock>()` + `T = abs_art_tokio::Runtime<FULL>`
  （tokio 计时器）+ `C = WallClock`（`type Instant = std::time::Instant`）**编译通过**——
  两条约束互不相干，前文 §3.1 担心的情况照旧。
- `[实测]` 运行期后果（p3 e1，复现前文 §3.1）：`start_paused = true` 下墙上时钟版空闲
  超时跑 3 轮、虚拟时间推进 ≥900s、墙上 <1s、`fired == false`；换成
  `tokio::time::Instant` 第一轮就 `fired == true`。
- `[实测]` **能拦住的形状**：`C: TrClock<Instant = T::Instant>`——类型不符时 `E0271`
  （p3 d2/d3）。代价是 `TrDelay` / `TrTime` 必须暴露 `Instant` 关联类型，与前文
  `time-20261005-1225.md`「刻意不暴露 `Instant`」的决策**冲突**，须重新裁决。
- **残留洞**（这条最关键）：类型相同 ≠ 同源。`type Instant = tokio::time::Instant` 但用
  `Instant::from_std(std::time::Instant::now())` 取时刻**仍然合法**，虚拟时间下偏差
  ≥500s（p3 e1 的同类型不同源用例）。因此「缺省必须与后端计时同源」只能靠
  **后端提供缺省值**来保证，类型约束钉不住；若要彻底堵死，只能让「等待」与「时刻」由
  同一个 trait 提供（注入点随之消失，前文 §3.5 的可注入性也就没了）。

---

## 4. 建议顺序与裁决项

| 顺序 | 项 | 独立性 | 建议 |
| --- | --- | --- | --- |
| 1 | 想法 3：`TrClock` + `CLOCK` 位 | 独立 | 先做「运行时类型上的 `TrClock` + 新能力位」；「是否暴露 `Instant` 关联类型」单独裁决（与不暴露 `Instant` 的旧决策冲突） |
| 2 | 想法 1：**改由 bridge 承载默认 runtime**（§6） | 独立 | 推荐这条路：`abs_art` 零改动、库不被 feature 传染、顺带给想法 2 提供能力表宿主。若还想让 `abs_art` 自己拥有 `Runtime` 符号，再叠加形态③投影（§1.5），**不要**以「让业务代码少写参数」为理由做形态②（代价大且做不到目标） |
| 3 | 想法 2：thread-local `LocalScope` | 中（探针已绿） | 机制已实测可行（tokio/smol），能力表宿主按 §6 放 bridge 即可；落地前须接受代价：驱动契约变严且静默失败、测试隔离要自己还、失去编译期跨线程保证 |

三条之间的耦合（本轮的最终判断）：**§6 的 bridge 承载方案同时给出了想法 1 的落点与想法 2
的能力表归宿**——bridge 既是「唯一默认 runtime」的符号来源，也是 `thread_local!` 的宿主；
想法 3 只需在 `abs_art` 加 `TrClock` 与 `CLOCK` 位，bridge 的 `now()` 走默认后端即天然同源。
于是三件事可以合成一条线：**抽象层只留 trait 与能力位，具体与默认都由 bridge 承担。**

---

## 5. 证据索引

| 编号 | 结论 | 命令 | 结果 |
| --- | --- | --- | --- |
| E1 | 形态①不可行 + 孤儿规则边界 | `cargo check -p app` / `-p ab2_*`（`p1_orphan`） | `E0117`；`ab2_traitparam` 为 `EXIT=0` |
| E2 | 固有 impl 写不了 | `cargo check -p fail_inherent` | `E0116` |
| E3 | 抽象层自带 tag 也不行 | `cargo check -p fail_foreign_tag` | `E0117` |
| E4 | 形态②最小同构可用 | `cargo run -p app_bb` | `EXIT=0` |
| E5 | 下游写不出竞争 impl | `cargo check -p conflict` | `E0117` |
| E6 | 抽象层默认 `B` 不可用 | `cargo check -p fail_default_aa` | `E0277` |
| E7 | tag 形态 bridge 两后端同存 | `cargo run -p app_bridge2` | `EXIT=0` |
| E8 | 抽象层 feature 依赖后端 → 环 | `cd cyclic && cargo check -p cyc_a --features backend-b` | `cyclic package dependency` |
| E9 | 唯一能给出默认 `B` 的形状 | `cargo run -p app_one` | `EXIT=0`（后端并进抽象层） |
| E10 | 形态①合法部分（别名/自由函数/扩展 trait） | `cargo run -p app_ext` | `EXIT=0` |
| E11 | 旧形态 bridge 两后端同存 | `cargo run -p app_old_bridge` | `EXIT=0` |
| E12 | 形态②六条 blanket impl（真实 trait） | `cargo test -p art_rt`（`p2_tagged`） | 6 passed，`EXIT=0` |
| E13 | 形态②端到端（真 tokio） | `cargo run -p app`（`p2_tagged`） | `P2-APP-OK`，`EXIT=0` |
| E14 | 形态② bridge 双后端 | `cargo check -p bridge --features backend-compio` | `EXIT=0` |
| E15 | 形态②能力位门控仍生效 | `cargo check -p badcaps` | `E0277` |
| E16 | 形态③投影（含 GAT 投影） | `cargo run -p app_cc` | `EXIT=0` |
| E17 | 形态③投影 + **真实 abs_art trait** | `cargo run -p proj_app` | `EXIT=0` |

| E18 | 形态②端到端全绿（真 tokio，26 tests） | `cargo test`（`p2_tagged`） | 26 passed；`cargo clippy --all-targets` 0 warning |
| E19 | 形态②唯一设计坑：`BackendTime` 漏 `: BackendDelay` | `cargo check -p supertrait_trap` | `E0277`，修法 = 补超 trait（零语义变化） |
| E20 | 旧形态双后端同开 | `cargo check -p e0252_contrast` | `E0252`（对照） |
| E21 | `TrClock` 四种时刻类型全部满足约束 | `cargo test -p clock_ok`（`z_clock`） | 1 passed，`EXIT=0`；非空由 `clock_fail`（`f32`）`E0271` 钉住 |
| E22 | 6 位能力位表可由宏生成 | `cargo test -p z_caps6`（`p1_orphan`） | 1 passed，`EXIT=0`（192 个 impl） |
| E23 | `generic_const_exprs` 不可用 | `p3_clock` b5/b6/b8 | nightly 带 warning；**负例 ICE**；stable `E0554` |
| E24 | 「同源」反例（编译通过）与运行期后果 | `p3_clock` d1 / e1 | 墙上时钟版 `fired == false`；tokio 时钟版第一轮即 `fired == true` |
| E25 | 想法 2 最终全绿（修好脚手架后） | `cd p2_tls && cargo test --workspace` | tokio 19 passed / smol 17 passed / doctests 7 passed；串行与并行均 `EXIT=0` |

（`evidence/*.log` 在各探针目录下；`p1_orphan/README.md` 有逐项复现清单。）

---

## 6. 追加讨论：让 `abs_art-bridge` 当「唯一默认 runtime」

> 本节回应一个更贴近目标的问题：想法 1 的**目的**不是「少写一个类型参数」，而是
> 「让使用者像已经决定好了一个 runtime 那样工作」。既然 `abs_art` 承载不了（§1.6 的
> 不可能三角），**能不能由 `abs_art-bridge` 承载？**

### 6.1 提案形状

1. bridge 用 Cargo feature 决定**实际链接哪些后端**；**多个可同开**（取消「只能启用一个」
   的 `compile_error!`），**零个**时仍 `compile_error!`（fail fast）。
2. bridge 给出**全局唯一的默认符号**：`pub type Runtime<const CAPS: usize = FULL> = <按 cfg
   优先级选中的后端 Runtime>`，并另给**具名别名**（`TokioRuntime` / `CompioRuntime`）供
   「同一二进制里要两个后端」的场景。
3. bridge 再给**自由函数面**：`block_on` / `delay` / `interval` / `timeout` / `spawn` /
   `spawn_blocking` / `now` / `spawn_local` / `run_until`——调用点看不到后端类型名。
4. 下游库以 `default-features = false` 依赖 bridge（**零后端 feature**），自己的测试用
   `[dev-dependencies]` 再依赖一次并开一个后端；**应用**决定最终后端。
5. bridge 是 std crate，因此顺带成为**想法 2 的能力表宿主**；默认时钟走默认后端，
   想法 3 的「缺省同源」在默认路径上**由构造保证**。

### 6.2 实测：feature 传染是否真的消失（`p4_bridge_default/`，模型后端）

用模型后端（两个本地 `Runtime<CAPS>` + 一份本地 `TrDelay`）把注意力集中在 Cargo 的
feature 统一语义上：

| 验证点 | 命令 | 结果 |
| --- | --- | --- |
| 库单独 `check`（普通依赖零后端） | `cargo check -p lib_consumer` | bridge 的 `compile_error!`，`EXIT=101` |
| 库单独 `test`（dev-dep 给 tokio） | `cargo test -p lib_consumer` | `library_tests_can_pin_a_backend_via_dev_dependencies ... ok`，`EXIT=0` |
| 应用选 tokio | `cargo run -p app_tokio` | `bridge::tag() = tokio, lib_consumer::describe() = tokio` |
| **应用选 compio（关键）** | `cargo run -p app_compio` | **`bridge::tag() = compio, lib_consumer::describe() = compio`** |
| 两后端同开 | `cargo run -p app_both` | 默认 = tokio（优先级），`CompioRuntime`/`TokioRuntime` 具名别名都在，默认睡眠是 `be_tokio::TokioSleep`（**具体类型**，静态分发） |
| 同上但用**旧 resolver v1** | `resolver = "1"` 下 `cargo run -p app_compio` | 仍 `compio / compio`，`EXIT=0`——dev-dep feature 同样不泄漏（非根包的 dev-deps 根本不会被激活） |

**关键结论：`[dev-dependencies]` 的 backend feature 没有泄漏进 normal build**——库跟着
**应用**的选择走（compio 构建里库看到 compio）。这正是「下游库不再承担 feature 传染的
心理负担」所需要的性质。

### 6.3 这个方案解决了什么、还剩什么

**解决**：库侧不必选后端、不必承担心传染（实测）；`abs_art` **零改动**（除想法 3 的
`TrClock`）；同一二进制里多后端同存不再需要 `compile_error!`；顺带给出想法 2 的能力表
归宿（bridge 是 std）。

**还剩（必须在文档/约定里写清）**：

1. **默认是全局且按优先级的**：同一构建里两个后端都链接时谁当默认由 bridge 的 cfg
   优先级决定，**应用无法覆盖**；需要特定后端的库必须用具名别名，不能走默认面。
2. **`bridge::Runtime` 是「随构建变化的类型」**：写进公开 API 后，rustdoc 与类型检查看到
   的是具体后端类型；库的公开 API 在不同构建下不同（可接受，但要写进约定）。
3. **能力位声明在自由函数面上消失**：`SPAWN_SEND` 那类位的价值是「必须写下来、可 grep」，
   而 `bridge::spawn(..)` 没有地方写。建议保留 `Runtime<CAPS>` 作**主面**、自由函数只作
   便捷面，否则 `caps.rs` 那套「强制显式」的设计意图会在这条路径上失效。
4. **零后端时库不能单独 `cargo check`**（本轮实测）。测试用 dev-dep 解决；若要让
   `cargo check -p lib` 也过，需要一个「stub 后端」（实现全部能力但 panic）的 feature，
   代价是可能带着假后端发布。
5. 库对 bridge 的依赖把「桥」放进每个下游的依赖图——**传染从「feature」换成了「依赖」**，
   只是后者不会强迫选择。
6. 「私有但全局唯一的 static 默认 runtime **对象**」在当前 API 形状下**只是符号**：
   `abs_art` 的能力全是**无 `self`** 的关联函数，拿到 `&'static Runtime` 也调不了能力
   （app_both 里必须自己加扩展 trait）。真正的 zero-cost 载体是**类型别名 + 自由函数**
   （静态分发、无 `dyn`、无装箱，§6.2 已断言返回具体类型）。

### 6.4 与想法 1/2/3 的关系

- 对想法 1：**它让形态③投影不再是必须的**——bridge 别名 + 自由函数已经给出「唯一 runtime」
  的体验，且 `abs_art` 不用动。若将来 `abs_art` 真有了 `Runtime<CAPS, B>`，bridge 只是把
  默认 tag 填进去，两者可无缝叠加。
- 对想法 2：它给出了前文 §2.3 (a)/(b) 的答案——**能力表放 bridge（(b)）**，不必给
  `abs_art` 开 std feature。
- 对想法 3：`abs_art` 只需加 `TrClock` + `CLOCK` 位；bridge 的 `now()` 走默认后端，
  默认路径天然同源；要注入假时钟时直接用某个具体类型/别名，绕开默认面。

---
