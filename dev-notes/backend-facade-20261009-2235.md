# `abs_art-facade`：把「当前后端」收敛到唯一一处

- 日期：2026-10-09 22:35
- 范围：`abs_art`（新增 crate）、`abs_buff_stdio_adapt`、`smux_v1`、
  `mptp_rpc/crates/{mptp_core, mptp_cs_demo}`、`smux_v1_sock_demo`

---

## 1. 要解决的问题：feature 只能向下流，且不可撤销

`abs_art-bridge` 有两套 feature：

- `backend-*` —— 链接哪个后端；
- `default-backend-*` —— 裸名（`Runtime` / `current`）指向谁。

Cargo 的 feature 是**全图并集、单调、不可撤销**的。于是：

1. bridge 自己的 `default = ["default-backend-compio"]` 会替**全图**占住「默认」这个
   位；下游想换 tokio 就会撞 bridge 的「只能声明一个默认后端」守卫（实测：两个
   `default-backend-*` 会让裸名**没有任何匹配的 cfg 分支**，直接找不到 `Runtime`）。
2. 唯一的解法是让**每一条**到 bridge 的边都写 `default-features = false`。漏一条就
   回到 compio；而这意味着依赖链上每个 crate 都要重复一遍「知道 bridge 的 feature」。
3. 更隐蔽的是**中间库自己的 `default = ["rt-compio"]` 也会点亮 `backend-compio`**——
   这是与 bridge 默认**并列**的第二类「会点亮后端」的边，关 bridge 默认并不能解决它。

结论：**「选择」无法沿依赖边接力传递；它必须是全局唯一的。** 需要被优化的是
「表达这个唯一选择有多麻烦」，而不是「能不能不表达」。

## 2. 设计：一个门面 + 非对称映射

新增 `abs_art-facade`，它是**全家族唯一**直接依赖 `abs_art-bridge` 的 crate，并且：

```toml
abs_art-bridge = { path = "../abs_art-bridge", default-features = false }

[features]
default   = ["rt-compio"]
rt-compio = ["abs_art-bridge/backend-compio"]          # 环境后端：只链接，不占默认位
rt-smol   = ["abs_art-bridge/default-backend-smol"]    # override：显式声明默认
rt-tokio  = ["abs_art-bridge/default-backend-tokio"]
```

**非对称是刻意的**，它同时满足四个此前互相打架的诉求：

| 诉求 | 靠什么满足 |
| --- | --- |
| 门面单独 `cargo build` 能过 | 缺省 `rt-compio` 只链一个裸 backend ⇒ 走 bridge 的「唯一 backend 即默认」分支 |
| 下游加一个 `rt-tokio` 就能换 | `rt-tokio` 用 `default-backend-tokio` **压过** compio，全图仍只有一个默认 |
| 下游**不需要**写 `default-features = false` | 缺省位没有被 compio 占死（它只链了裸 backend） |
| 想少链一个 compio 时仍可关 | 在**唯一决定点**写一次 `default-features = false, features = ["rt-tokio"]`（可选优化） |

配套的编译期断言（`src/lib.rs`）把「裸名 == 被选中的具名后端」钉死：

```rust
#[cfg(feature = "rt-tokio")]
const _: fn(TokioRuntime) -> Runtime = |rt| rt;   // fn(A)->B 只在 A==B 时成立
```

门面同时统一转出 `Runtime` / `LocalScope` / 三个具名别名、`ScopeHost` /
`TrRtCurrent` / `DefaultRt_` / `default_rt_`，以及 `abs_art` 那批 trait。
因为裸名现在**就是**被选中的后端，`ScopeHost` 只需要**一条**实现——此前
`smux_v1` 与 `abs_buff_stdio_adapt` 各自维护的「三选一 + 别名表」全部删除。

`ManualTime<R, C>` 的 `ScopeHost` 委托实现也搬进门面（孤儿规则：trait 在这里），
由 `mock-clock` feature 拉进可选依赖。

bridge 自己**完全不动**：它单独编译仍走 compio 默认，「独立可编」与门面无关。

## 3. 迁移后的传导形状

```
abs_art-bridge            ← 只有 abs_art-facade 一条边（default-features = false）
    ▲
abs_art-facade            ← 映射 rt-* ⇒ backend-* / default-backend-*
    ▲        ▲          ▲
    │        │          └── mptp_core ── mptp_cs_demo  ← 唯一选择点：rt-tokio
    │        └── smux_v1
    └── abs_buff_stdio_adapt
```

- 每个 crate 只写 `rt-* = ["abs_art-facade/rt-*"]`（库的缺省=compio），**不再逐层转发
  `abs_buff_stdio_adapt/rt-*`**——门面的选择是全图并集的，底层自动跟上。
- `smux_v1` 保持公开路径：`smux_v1::connection::{ScopeHost, TrRtCurrent, DefaultRt_,
  default_rt_}` 改为门面的 **re-export**；`x_deps::abs_art_bridge` 换成
  `x_deps::abs_art_facade`。

## 4. 踩到的坑：`[patch]` 只对构建根生效 ⇒ 两个 `abs_art` 实例

门面在联调期是**本机 path** 依赖，因此它带进来的是 path 版 `abs_art` 家族；而图上仍有
若干 crate 以 git spec 依赖 `abs_art`：

- `smux_v1` → `abs_art` / `abs_art-mock_clock`；
- `buffex_tokio_adapt` → `abs_art-tokio`；`buffex_smol_adapt` → `abs_art-smol`。

`[patch]` **只对构建根 workspace 生效**，`smux_v1` 自己 manifest 里那份对作为依赖的它
无效。结果：`mptp_cs_demo` 图里出现两个 `abs_art-tokio`。

**症状极具迷惑性**：编译通过；`just test`（进程内环回）也通过；但 `just pairs` 里两端
TCP 连接 ESTABLISHED、双向**零字节**，握手永久停住——

```
ESTAB 127.0.0.1:34581 ←→ 127.0.0.1:60214
服务端 park（futex_wait），客户端 io_cqring_wait（执行器空闲）
```

原因是 `buffex_tokio_adapt` 的写泵挂到了 **git 版** `abs_art-tokio` 的本地队列上，而
连接循环跑在**path 版**的队列里——两个 crate 实例，谁也不驱动谁。

修法：每个**构建根**打开 `[patch."…abs_art.git"]`，把图上真正来自 git 的包
（`abs_art` / `abs_art-mock_clock` / `abs_art-tokio` / `abs_art-smol`）重定向到本地
path。`abs_art-bridge` 不列（门面走 path）、`abs_art-compio` 在 demo 图上无 git 来源
（`buffex_compio_adapt` 不依赖 `abs_art`）。涉及 `mptp_cs_demo`、`mptp_rpc` 工作区根、
`smux_v1`、`smux_v1_sock_demo` 四处。

诊断命令：

```bash
cargo tree --offline --no-default-features --features rt-tokio -d   # 看重复包
cargo tree --offline --no-default-features --features rt-tokio | grep 'abs_art.*(https'  # 看残留 git 源
```

## 5. 验证（全部实跑）

| 项目 | 结果 |
| --- | --- |
| `abs_art` 门面：缺省 / `--features rt-tokio` / `--no-default-features --features rt-{tokio,smol,compio}` / `mock-clock` | 全部编过，编译期断言通过 |
| `abs_art` 工作区 `cargo check --workspace` | 通过 |
| `abs_buff_stdio_adapt` `cargo test` | 14 passed（走门面的 compio 缺省） |
| `smux_v1` 五格 `--all-targets`（缺省/tokio/compio/smol/metrics） | 0 error |
| `smux_v1` `just test`（无参数，含 clippy + 三装配 + metrics） | 通过 |
| `mptp_core` `cargo test` | 4 + 1 doctest passed |
| `mptp_cs_demo` `just test` | 通过（含进程内环回 200 OK） |
| `mptp_cs_demo` `just pairs` | **3/3 通过**（tokio→compio、tokio→smol、compio→smol） |
| `smux_v1_sock_demo` `just build` + `just test` | 通过（12 passed） |

## 6. 遗留

- `mptp_rpc/crates/mptp_smux_iroh` **在 HEAD 上就编不过**（`pub use mptp_rpc_core;`，
  而该 crate 并不是依赖）。与本次门面无关，未动。
- 三个设备适配器（`buffex_*_adapt`）仍以 git spec 依赖 `abs_art` / `abs_art-tokio` /
  `abs_art-smol`；等 `abs_art-facade` 推到 gitee 后，可以把这些 crate 改为依赖门面并
  去掉各构建根的 `[patch]`。届时「唯一连 bridge」的纪律才真正闭环。
