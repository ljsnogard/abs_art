//! `abs_art-demo` 二进制：负责创建运行时、构造运行时**值**、取得**作用域**
//! 并调用业务库。
//!
//! 后端选择发生在 `Cargo.toml`（`demo-tokio` / `demo-compio` feature）：
//!
//! - 默认 `demo-tokio`：创建 tokio 运行时；
//! - `--no-default-features --features demo-compio`：创建 compio 运行时。
//!
//! 本文件是唯一允许感知后端的地方：创建哪个运行时的代码必须与所选后端一致；
//! 业务库（`src/lib.rs`）在两种后端下零改动。
//!
//! # 集成方的四步（运行时值 + 作用域两件套）
//!
//! 1. **创建运行时**（后端特有：tokio 的 `Runtime::new()` / compio 的
//!    `Runtime::new()`）；
//! 2. **构造运行时值**：在后端上下文内构造声明了能力的类型别名值
//!    （`current()` 需要上下文，见 `strict_mode_check::no_context_construction`）；
//! 3. **取得作用域**：`value.local_scope()`——它是取得本地投递能力的**唯一
//!    入口**，要求 `CAPS` 含 `SPAWN_LOCAL`；
//! 4. **把两件套交给业务函数**：计时用 `&value`、投递用 `&scope`。
//!
//! 第 2、3 步是「值 + 作用域」分工带来的：业务函数不再自己「凭空」取得运行时，
//! 也不再假设计时与本地队列来自同一个对象——两者由集成方显式传递。

/// tokio 演示组：用 tokio 运行时驱动业务库。
///
/// tokio 组能额外演示依赖 `TrSpawnSend` 的路径（`CapRt` + `double_via_runtime`），
/// 因为 tokio 有真正的跨线程全局工作队列。
#[cfg(feature = "demo-tokio")]
fn main() {
    // 第 1 步：创建后端运行时（这里唯一感知 tokio 的地方）。
    let rt = tokio::runtime::Runtime::new().unwrap();

    // 第 2~4 步：构造值、取作用域、交给业务函数。
    let (send_out, local_out) = rt.block_on(async {
        // 跨线程投递路径：声明 `BLOCK_ON | SPAWN_SEND` 的运行时值。
        let send_value = abs_art_demo::CapRt::current();
        let send_out = abs_art_demo::double_via_runtime(&send_value, 21);

        // 本地投递路径：声明 `BLOCK_ON | DELAY | SPAWN_LOCAL` 的运行时值，
        // 由它交出作用域；计时仍从值上取。
        let local_value = abs_art_demo::LocalRt::current();
        let scope = local_value.local_scope();
        let local_out = abs_art_demo::timed_local_double(&local_value, &scope, 21).await;

        (send_out, local_out)
    });

    assert_eq!(send_out, 42, "spawn + block_on");
    assert_eq!(local_out, 42, "delay + spawn_local + run_until");
    println!("abs_art-demo (tokio backend) OK: send={send_out}, local={local_out}");
}

/// compio 演示组：用 compio 运行时驱动业务库。
///
/// compio 的 `Runtime::new()` 默认开启全部 driver（含 time），且运行时是
/// 线程本地的，因此这里不需要像 tokio 那样选 multi-thread / enable_all。
///
/// compio **没有**跨线程全局工作队列，它的运行时值不实现 `TrSpawnSend`，因此本
/// 分支不演示 `spawn`，而是演示替代路径：把同一份「多任务聚合」业务放进本地
/// 作用域投递（见 [`abs_art_demo::local_three_tasks`]）。
#[cfg(feature = "demo-compio")]
fn main() {
    // 第 1 步：创建后端运行时（这里唯一感知 compio 的地方）。
    let rt = compio::runtime::Runtime::new().unwrap();

    // 第 2~4 步：构造值、取作用域、交给业务函数。
    let (local_out, local_tasks_out) = rt.block_on(async {
        // 本地投递路径：计时从值上取、投递从作用域上取。
        let value = abs_art_demo::LocalRt::current();
        let scope = value.local_scope();
        let local_out = abs_art_demo::timed_local_double(&value, &scope, 21).await;

        // `TrSpawnSend` 的替代路径：同一份业务改用本地作用域投递。
        let full_value = abs_art_demo::FullRt::current();
        let full_scope = full_value.local_scope();
        let local_tasks_out = abs_art_demo::local_three_tasks(&full_scope, 7).await;

        (local_out, local_tasks_out)
    });

    assert_eq!(local_out, 42, "delay + spawn_local + run_until");
    assert_eq!(local_tasks_out, 42, "7 + 14 + 21（本地作用域投递）");
    println!(
        "abs_art-demo (compio backend) OK: local={local_out}, local_tasks={local_tasks_out}"
    );
}
