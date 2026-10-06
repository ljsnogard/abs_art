//! 测试骨架：把每个用例放到独立线程里执行并加超时。
//!
//! # 为什么必须加超时
//!
//! `spawn_local` 的后端语义差异，表现形式往往是**宿主被永久阻塞**（循环任务
//! 没人驱动，宿主等一个永远不会到的回执），而不是返回一个错误。若不隔离线程并
//! 设超时，一个后端的失败会让整个测试进程挂死，拿不到任何可读结论。
//!
//! 隔离线程还有一个附带好处：`!Send` 的运行时**值**都在**闭包内部**构造和使用，
//! 不跨越线程边界，因此不会给用例本身引入额外的 `Send` 约束。
//!
//! # 值化之后，「在闭包内部构造」从便利变成了硬要求
//!
//! v0.3 里 `!Send` 的只是那个**独立的作用域对象**（tokio 的 `LocalSet`、compio 的
//! `LocalScope`、`smol::LocalExecutor`），运行时类型本身还是 ZST，随处可写。
//!
//! v0.4 把本地队列并进运行时**值**之后，`!Send` 的正是运行时值本身（tokio 的
//! `Runtime` 持 `Rc<LocalSet>`、smol 的 `Runtime` 持 `Rc<LocalExecutor>`）。于是
//!
//! - 运行时值**不可能**在别处造好再搬进闭包——`F: Send` 的约束会直接拒绝；
//! - 唯一可行的写法就是「闭包内 `current()` / `with_handle(..)` 造值 → 立刻在本
//!   线程用它投递与驱动」，也就是每个用例骨架现在的形状。
//!
//! 换句话说，本骨架不仅挡「宿主被永久阻塞」，也顺带保证了「运行时值不跨线程」
//! 这条值化后的结构约束不会被无声违反。

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
/// `f` 需要自行创建运行时**值**并在其中驱动 future（因此要求 `Send + 'static`）；
/// 用例自身的失败经 `f` 返回的 `Result` 透传，超时则在这里统一成一条
/// 指向「任务没有被驱动」的错误信息。
///
/// 运行时值本身是 `!Send` 的（见模块文档），所以它只能出现在 `f` 的**函数体**里，
/// 不能作为 `f` 的捕获项或返回值。
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
