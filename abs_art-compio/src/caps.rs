//! 本后端的能力位：**完整能力集** [`FULL`] 与「声明了 `SPAWN_SEND` 就一用即报错」的
//! 静态断言 [`CompioCaps_`]。
//!
//! # 为什么光「不实现 [`TrSpawnSend`](abs_art::TrSpawnSend)」还不够
//!
//! `abs_art` 的能力位是一张**位集合**表：[`abs_art::FULL`] 是 6 位全置的 `63`，它回答的
//! 是「这一位在不在集合里」，而不是「本后端能不能兑现这一位」。compio 兑现不了
//! [`SPAWN_SEND`]（投递到跨线程全局工作队列）：它的执行器是 `Rc<Executor>`，绑在创建
//! 运行时的那条线程上，`Runtime::spawn` 投的就是这条线程自己的队列（完整因果见
//! [`crate::spawn_send`]）。
//!
//! 如果只做到「不写 `impl TrSpawnSend`」，那么
//! `Runtime::<{ SPAWN_SEND }>::current()` 依然能构造出一个值，错误要一直拖到调用
//! `spawn` 时才以「没有 `spawn` 方法」的形式暴露——**离病因很远**。
//!
//! 本模块把失败**提前到「值被使用」这一刻**：
//!
//! ```text
//! 声明了 SPAWN_SEND 的 Runtime<CAPS> 值一被使用（构造 / 调能力方法 / 作为泛型实参）
//!         ↓
//! [(); CAPS]: CompioCaps_ 不成立
//!         ↓
//! #[diagnostic::on_unimplemented] 给出人话错误
//! ```
//!
//! 于是 `Runtime::<{ abs_art::FULL }>::current()`（`abs_art::FULL == 63` 含 bit2）会在
//! **构造点**就报：
//!
//! ```text
//! error[E0277]: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力
//! ```
//!
//! # 门控范围与落点：为什么必须写在 `Runtime` 的**类型定义**上
//!
//! 断言首先写在 `Runtime` 自己的 `where [(); CAPS]: CompioCaps_` 上，然后**重复**在
//! 每一个 impl 上（Rust 要求每个 impl 都能证明类型定义的 where 子句）：
//!
//! - 类型定义：`pub struct Runtime<const CAPS: usize = FULL> where [(); CAPS]: CompioCaps_`；
//! - 固有入口：`current` / `with_runtime` / `retag` / `tag` / `runtime` / `local_scope`，
//!   外加 `Clone`（它内部走 `retag`）；
//! - 能力 trait：[`TrBlockOn`](abs_art::TrBlockOn) / [`TrDelay`](abs_art::TrDelay) /
//!   [`TrClock`](abs_art::TrClock) / [`TrTime`](abs_art::TrTime) /
//!   [`TrSpawnBlocking`](abs_art::TrSpawnBlocking) / [`TrAsyncRuntime`](abs_art::TrAsyncRuntime)。
//!
//! **实测**：断言**只**挂在固有 impl 上时，
//! `Runtime::<{ abs_art::FULL }>::current()` 得到的是
//! ``error[E0599]: the associated function or constant `current` exists for struct
//! `abs_art_compio::Runtime<63>`, but its trait bounds were not satisfied``——里面**看不到**
//! `#[diagnostic::on_unimplemented]` 的文案（`E0599` 这条路径不查该属性），人话信息
//! 等于白写。把它写进**类型定义的 where 子句**之后，rustc 先报一条 `E0277`，正是那条
//! 人话信息（随后另附一条 `E0599` 作为次生错误，不影响可读性）。
//!
//! 这样一来，「泛型代码里写 `R: TrDelay`、实参却给出 `Runtime<{SPAWN_SEND}>`」也会在
//! 实参处报同一条信息，而不是等到调用点。

use abs_art::{FULL as ABS_ART_FULL, SPAWN_SEND};

/// 本后端的**完整能力集**：不含 [`SPAWN_SEND`]（compio 没有跨线程全局队列）。
///
/// 它是 [`Runtime`](crate::Runtime) 的默认 `CAPS`，也是「compio 能兑现的全部能力」的
/// 唯一定义：`BLOCK_ON | DELAY | SPAWN_LOCAL | SPAWN_BLOCKING | CLOCK`（`59`）。
///
/// # 与 [`abs_art::FULL`] 的区别
///
/// [`abs_art::FULL`] 是**位集合**意义上的「全部」（`63`，含 `SPAWN_SEND`），它不代表
/// 任何单个后端都实现得了。本常量由它**去掉 `SPAWN_SEND` 位**得到，因此：
///
/// ```
/// use abs_art_compio::{FULL, Runtime};
///
/// assert_eq!(FULL, abs_art::FULL & !abs_art::SPAWN_SEND);
/// // 默认 CAPS 就是本后端的 FULL：拿到的是能用的值
/// let rt = compio::runtime::Runtime::new().unwrap();
/// let value = rt.block_on(async { abs_art_compio::current() });
/// let _same_type: Runtime<{ FULL }> = value;
/// ```
pub const FULL: usize = ABS_ART_FULL & !SPAWN_SEND;

/// 静态断言：compio 的运行时值**不允许**声明 [`SPAWN_SEND`]。
///
/// 本 trait 只对 `0..=63`（6 位能力表）中**不含 `SPAWN_SEND` 位**的 32 个掩码实现，
/// 因此任何含该位的 `CAPS`（例如 [`abs_art::FULL`] `== 63`，或裸的 [`SPAWN_SEND`]
/// `== 4`）都会在「值被使用」处触发下面的错误信息。
///
/// # 为什么是「静态失败」而不是「少实现一个 trait」
///
/// 位被声明 = 调用方声称「我依赖一条可跨线程共享的队列」。compio 上这个前提不存在，
/// 与其让问题拖到 `spawn` 调用点（那里只会说「没有 `spawn` 方法」），不如在**值一出现**
/// 时就拒绝。这也是能力位纪律「必须写下来」的另一面：写错了要立刻知道。
///
/// # 错误信息（`#[diagnostic::on_unimplemented]`，Rust 1.78+ 稳定）
///
/// 第一条：`abs_art::FULL`（`63`）含 `SPAWN_SEND` 位 →
/// `error[E0277]: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力`
///
/// ```compile_fail
/// use abs_art_compio::Runtime;
///
/// // `abs_art::FULL == 63`：bit2（`SPAWN_SEND`）置位 → 构造点即编译失败（E0277）
/// let rt = compio::runtime::Runtime::new().unwrap();
/// rt.block_on(async {
///     let _value = Runtime::<{ abs_art::FULL }>::current();
/// });
/// ```
///
/// 第二条：只声明 `SPAWN_SEND`（`4`）也一样 →
/// `error[E0277]: compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力`
///
/// ```compile_fail
/// use abs_art::SPAWN_SEND;
/// use abs_art_compio::Runtime;
///
/// // 位被声明 = 声称要跨线程全局队列；compio 没有 → 编译失败（E0277）
/// let rt = compio::runtime::Runtime::new().unwrap();
/// rt.block_on(async {
///     let _value = Runtime::<{ SPAWN_SEND }>::current();
/// });
/// ```
///
/// 作为对照，去掉这一位之后一切照常（本后端的完整能力集就是 [`FULL`]）：
///
/// ```
/// use abs_art::{BLOCK_ON, SPAWN_BLOCKING, TrBlockOn};
/// use abs_art_compio::Runtime;
///
/// let rt = compio::runtime::Runtime::new().unwrap();
/// // 只要不含 SPAWN_SEND 位，掩码随便组合都能用（`block_on` 另需 BLOCK_ON 位）
/// let out = rt.block_on(async {
///     let value = Runtime::<{ BLOCK_ON | SPAWN_BLOCKING }>::current();
///     value.block_on(async { 40 + 2 })
/// });
/// assert_eq!(out, 42);
/// ```
///
/// # 它不会被下游「顺手补上」
///
/// `[(); M]` 是数组类型（外来类型）、`CompioCaps_` 是本 crate 的 trait，因此下游 crate
/// 为某个掩码补 impl 会撞上孤儿规则 `E0117`：这条断言是**密封**的。
#[diagnostic::on_unimplemented(
    message = "compio 后端没有 `SPAWN_SEND`（跨线程全局工作队列）能力",
    label = "请从 CAPS 中去掉 `abs_art::SPAWN_SEND`；compio 的完整能力集是 `abs_art_compio::FULL`",
    note = "compio 的执行器是线程本地的：`Runtime::spawn` 投的是本线程运行时的队列。要投递任务请用 `Runtime::local_scope()` 的 `spawn_local`。",
    note = "本断言只对 `0..=63` 中不含 `SPAWN_SEND` 位的 32 个掩码成立；若 CAPS 里还有 `0..=63` 之外的位置位，同样会看到这条信息。"
)]
pub trait CompioCaps_ {}

macro_rules! impl_compio_caps_ {
    ([$($mask:literal),* $(,)?]) => {
        $(impl CompioCaps_ for [(); $mask] {})*
    };
}

// 0..=63（6 位能力表）中**不含 `SPAWN_SEND`** 的全部 32 个掩码，判据只有一条：
// `mask & SPAWN_SEND == 0`，即 bit2 清零。等价地，就是
// {BLOCK_ON, DELAY, SPAWN_LOCAL, SPAWN_BLOCKING, CLOCK} 五个位的全部子集
// （2^5 = 32 个）——`FULL == 59` 也在其中。
//
// 注意**不要**漏掉 16..=19（只声明 `SPAWN_BLOCKING` 等组合）：compio 兑现得了
// `spawn_blocking`，把它们挡在外面是假阴性。反过来**不要**放进 60..=63：它们的
// bit2 是 1（含 `SPAWN_SEND`），放进来会让本模块的目的落空，并直接与
// 「`Runtime::<{ abs_art::FULL }>::current()` 必须编译失败」冲突。
impl_compio_caps_!([
    0, 1, 2, 3, 8, 9, 10, 11, 16, 17, 18, 19, 24, 25, 26, 27, 32, 33, 34, 35, 40, 41, 42, 43, 48,
    49, 50, 51, 56, 57, 58, 59,
]);

#[cfg(test)]
mod tests {
    //! [`FULL`] 与 [`CompioCaps_`] 掩码表的单元测试。

    use super::*;

    /// 编译期断言辅助：`T` 必须实现 [`CompioCaps_`]（不满足则编译失败）。
    fn assert_compio_caps_<T: CompioCaps_>() {}

    /// 目的：验证本后端的 [`FULL`] 恰好是「`abs_art::FULL` 去掉 `SPAWN_SEND` 位」，
    /// 且它本身通过静态断言（默认 CAPS 一定可用）。
    ///
    /// 手段：断言数值关系 `FULL == abs_art::FULL & !SPAWN_SEND` 与具体值 `59`，并把
    /// `[(); FULL]` 交给编译期断言函数。
    ///
    /// 判断：三条断言全部成立（其中第三条是编译期检查）即为通过；若 `abs_art` 挪动了
    /// 位号，第一条与 `59` 这条会一起报警。
    #[test]
    fn full_is_all_caps_without_spawn_send() {
        assert_eq!(FULL, ABS_ART_FULL & !SPAWN_SEND);
        assert_eq!(FULL, 59);
        assert_compio_caps_::<[(); FULL]>();
    }

    /// 目的：钉住「不含 `SPAWN_SEND` 位的 32 个掩码全部被允许」——既防漏（`16..=19`
    /// 这类只声明 `SPAWN_BLOCKING` 的合法组合被误挡），也防多（`60..=63` 这类含
    /// `SPAWN_SEND` 的掩码被误放）。
    ///
    /// 手段：把 32 个掩码逐个交给编译期断言函数 `assert_compio_caps_`；列表之外
    /// 的掩码（含 `SPAWN_SEND` 位）**不能**出现在这里——它们一旦也被实现，本模块的
    /// `compile_fail` 文档测试就会失败。
    ///
    /// 判断：编译通过即为通过（列表若少一项，测试无法编译；若多一项含 `SPAWN_SEND`
    /// 的掩码，文档测试会失败）。
    #[test]
    fn all_masks_without_spawn_send_are_allowed() {
        assert_compio_caps_::<[(); 0]>();
        assert_compio_caps_::<[(); 1]>();
        assert_compio_caps_::<[(); 2]>();
        assert_compio_caps_::<[(); 3]>();
        assert_compio_caps_::<[(); 8]>();
        assert_compio_caps_::<[(); 9]>();
        assert_compio_caps_::<[(); 10]>();
        assert_compio_caps_::<[(); 11]>();
        assert_compio_caps_::<[(); 16]>();
        assert_compio_caps_::<[(); 17]>();
        assert_compio_caps_::<[(); 18]>();
        assert_compio_caps_::<[(); 19]>();
        assert_compio_caps_::<[(); 24]>();
        assert_compio_caps_::<[(); 25]>();
        assert_compio_caps_::<[(); 26]>();
        assert_compio_caps_::<[(); 27]>();
        assert_compio_caps_::<[(); 32]>();
        assert_compio_caps_::<[(); 33]>();
        assert_compio_caps_::<[(); 34]>();
        assert_compio_caps_::<[(); 35]>();
        assert_compio_caps_::<[(); 40]>();
        assert_compio_caps_::<[(); 41]>();
        assert_compio_caps_::<[(); 42]>();
        assert_compio_caps_::<[(); 43]>();
        assert_compio_caps_::<[(); 48]>();
        assert_compio_caps_::<[(); 49]>();
        assert_compio_caps_::<[(); 50]>();
        assert_compio_caps_::<[(); 51]>();
        assert_compio_caps_::<[(); 56]>();
        assert_compio_caps_::<[(); 57]>();
        assert_compio_caps_::<[(); 58]>();
        assert_compio_caps_::<[(); 59]>();
    }
}
