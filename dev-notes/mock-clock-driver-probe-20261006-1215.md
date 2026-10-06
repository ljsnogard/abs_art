# mock clock 驱动策略探针：三后端能不能「空闲即推进」

日期：2026-10-06 12:15
分支：`feat/abs_art-runtime`
性质：**可行性实测 + 架构定型**（本轮只做探针，生产代码待批准后落地）
探针：`abs_art_runtime_probe/p9_mock_drive/`（1 个 crate + 3 个 bin + 9 份证据日志）

---

## 0. 结论

**成立，且三后端可以用同一个驱动形状**：一个包住测试主体的 `Supervisor` future
——外层 future Pending 时依次做「tick 执行器 → 推进虚拟时钟 → 自唤醒重进」。

| 后端 | `block_on` | 执行器 tick 钩子 | 实测 |
| --- | --- | --- | --- |
| smol | `smol::block_on(..)` | `LocalExecutor::try_tick()` | ✅ 虚拟时刻 = 1000ms |
| compio | `Runtime::block_on(..)` | `Runtime::run()` | ✅ 虚拟时刻 = 1000ms |
| tokio | `Runtime::block_on(LocalSet::run_until(..))` | **不需要**（`block_on` 自己会驱动任务） | ✅ 虚拟时刻 = 1000ms |

顺带白拿一条好处：supervisor 能把**真死锁**（既没有就绪任务、也没有可推进的定时器）
变成响亮的 panic，而不是静默挂起。

## 1. 三个把设计逼出来的坑（都是实测）

### 1.1 compio：外部 `while` 循环 + `run()` 会死锁

第一版用「外部循环：`if !rt.run() { clock.advance_to_next_() }`」，smol 通、compio 挂：

```text
[smol] 自动推进成功：3 轮，虚拟时刻 = 1000ms
  [诊断] 第 2 轮：run()=false，下一个 deadline=None，now=1000ms
thread 'main' panicked at ...: 没有就绪任务、也没有定时器可推进 → 死锁（run()=false, next=None）
```

**原因**（`compio-runtime-0.12.6/src/lib.rs:183-203` 的驱动循环）：

```rust
loop {
    if let Poll::Ready(result) = future.as_mut().poll(&mut context) { self.run(); return result; }
    let remaining_tasks = self.run();
    if remaining_tasks { self.poll_with(Some(Duration::ZERO)); } else { self.poll(); }
}
```

任务的 waker 唤醒的是 **driver**（`self.waker()`），所以只 tick 执行器不会把被唤醒的任务
重新调度；必须让 driver 的 `poll()` 再走一轮——而 `poll` / `poll_with` **不是公开 API**。
→ 结论：compio 的推进必须发生在**它自己的 `block_on` 循环内部**，这就是 supervisor 形状的由来。

### 1.2 tokio：时钟与标志是 `!Send`，`tokio::spawn` 编译不过

`E0277`（`Rc<Cell<_>>` 不能跨线程）→ 用 `LocalSet` + `spawn_local`，或把时钟做成
`Arc<Mutex<_>>`。两种都可行，选哪种取决于公开面（见 §3 待定项）。

### 1.3 tick 钩子不能用 trait，只能用闭包（`E0117`）

先写成 `trait Tick { fn tick_(&self) -> bool }` 放进探针库，让 bin 为
`smol::LocalExecutor` / `compio::runtime::Runtime` 实现它 → **`E0117`**（trait 与类型都外来）。
在真实布局里同样成立：`abs_art-mock_clock` 定义的 trait，后端 crate 也无法为
`tokio::runtime::Handle` 等外来类型实现。

→ **驱动钩子改用闭包**：`Supervisor::new(body, clock, || rt.run())`。
（另一条路是各后端自定义一个本地 newtype 包装运行时句柄，但闭包更省一层公开类型。）

## 2. 探针确认的形状（生产实现照此）

```rust
// abs_art-mock_clock（新 crate）
pub struct ManualClock { /* 时刻状态 + 到期唤醒表 */ }
impl ManualClock {
    pub fn now(&self) -> Instant64<..>;
    pub fn advance(&self, by: Duration);
    pub fn advance_to_next(&self) -> bool;      // 没有可推进的定时器 → false
    pub fn sleep(&self, d: Duration) -> MockDelay;   // 即 TrDelay::Delay 的实现
}

/// 装饰器：把任意运行时值的「时间」换成手动时钟，其余能力委托给 inner。
pub struct ManualTime<R> { /* inner: R, clock: ManualClock */ }
// impl TrDelay / TrClock / TrTime —— 同源由 ManualTime 自己保证
// impl TrSpawnSend / TrBlockOn / TrAsyncRuntime / … —— 委托给 inner
// impl TrMockClock —— advance / pause / resume（可被第三方扩展）

/// 驱动：Pending 时 tick → 推进 → 自唤醒；真死锁则 panic。
pub struct Supervisor<F, T: Fn() -> bool> { .. }
```

各后端以**可选 feature** 接入（`abs_art-{tokio,compio,smol}` 的 `mock-clock` feature）：
提供一个把 `block_on` + tick 闭包接好的驱动函数，并重导出 `abs_art-mock_clock`。

`Instant` 用 `embedded-timers` 的 `Instant64<FREQ>`：它是外来类型，但我们只是用
**关联类型**命名它（不实现任何 trait），所以没有 orphan 问题；实测它也满足
`TrClock::Instant` 的四条结构约束。

## 3. 待定（落地前需要定，因为都影响公开面）

1. **`ManualClock` 的线程模型**：`Rc<RefCell<_>>`（线程亲和，最省，但 tokio 侧必须用
   `LocalSet` + `spawn_local`）还是 `Arc<Mutex<_>>`（`Send + Sync`，三后端都能用
   `spawn`、可跨线程共享，代价是每次操作一次锁 + 需要 std 的 `Mutex`）？
2. **std / no_std**：`abs_art` 家族是 no_std；`abs_art-mock_clock` 是测试设施，
   先用 std 最直接（`std::sync::Mutex`）。若要保持 no_std，需要 `alloc` + 自旋锁
   （自己写 ~20 行 `unsafe`，或引入 `spin`）。
3. **crate 名**：`abs_art-mock_clock`（与 `abs_art-tokio` 等兄弟一致）还是你写的
   `abs_art-mock_clock` 字面形式？
