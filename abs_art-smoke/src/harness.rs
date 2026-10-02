//! 测试骨架：把每个用例放到独立线程里执行并加超时。
//!
//! # 为什么必须加超时
//!
//! `spawn_local` 的后端语义差异，表现形式往往是**宿主被永久阻塞**（循环任务
//! 没人驱动，宿主等一个永远不会到的回执），而不是返回一个错误。若不隔离线程并
//! 设超时，一个后端的失败会让整个测试进程挂死，拿不到任何可读结论。
//!
//! 隔离线程还有一个附带好处：`!Send` 的运行时对象（tokio 的 `LocalSet`、
//! compio 的 `Runtime`、`smol::LocalExecutor`）都在**闭包内部**构造和使用，
//! 不跨越线程边界，因此不会给用例本身引入额外的 `Send` 约束。

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// 单个用例的超时上限。
///
/// 取 5 秒：三个后端的正常路径都在毫秒级完成，5 秒足够容纳首次线程启动与
/// 运行时初始化的抖动，又能让失败快速可见。
pub const CASE_TIMEOUT: Duration = Duration::from_secs(5);

/// 在独立线程中执行 `f`；超时返回 `Err`。
///
/// `f` 需要自行创建运行时并在其中驱动 future（因此要求 `Send + 'static`）；
/// 用例自身的失败经 `f` 返回的 `Result` 透传，超时则在这里统一成一条
/// 指向「任务没有被驱动」的错误信息。
///
/// # Errors
///
/// - `f` 内部返回 `Err`：原样透传，并加上 `label` 前缀；
/// - `f` 在 [`CASE_TIMEOUT`] 内没有返回：返回超时错误。
pub fn run_case<T, F>(label: &str, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });

    match rx.recv_timeout(CASE_TIMEOUT) {
        Ok(inner) => inner.map_err(|e| format!("{label}：{e}")),
        Err(_) => Err(format!(
            "{label}：{CASE_TIMEOUT:?} 内未完成——宿主被永久阻塞，\
             说明任务没有被运行时驱动（本地队列没人推）"
        )),
    }
}
