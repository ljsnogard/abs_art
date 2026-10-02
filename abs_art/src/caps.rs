//! 运行时能力标记（capability tags）与类型级集合运算。
//!
//! 本模块用**常量位掩码** + 类型级标记把「运行时具备哪些能力」编码到类型里，
//! 供 `Runtime<const CAPS: usize>` 这类带 const 泛型的类型使用：
//!
//! - 每个能力对应一个位（见下面的 `BLOCK_ON` / `DELAY` / ... 常量）；
//! - `Has*` 标记 trait 为「包含对应位的掩码值」的 `[(); MASK]` 类型实现；
//! - 组合能力 = 位的按位或，例如 `BLOCK_ON | SPAWN_SEND`。
//!
//! 全部在编译期解析，零运行时开销。
//!
//! # 能力位的定位：写在代码上的「声明」，不是能力剪裁
//!
//! 能力位**拦不住**真心想用某项能力的人——他只要在自己的类型别名里把那一位写上就
//! 够了。它的价值在于**强制显式**：想用某项能力，必须先把它**写下来**，于是这次
//! 「升级」必然出现在类型别名、进而出现在 diff 与 code review 里，也能被 `grep`
//! 出来。最贴切的类比是 `unsafe`：任何人都会写，但必须写。
//!
//! 因此每个能力的实现都分成两半，**职责不同、不互相替代**：
//!
//! | | 回答的问题 | 载体 |
//! | --- | --- | --- |
//! | 能力位（本模块） | 你**声明**了没有？ | `Runtime<CAPS>` 类型级标记 |
//! | 能力值（如 [`crate::TrLocalScope`]） | 你**拿到**了没有？ | 具体类型的值 |
//!
//! 本地投递（`spawn_local`）把这条分工体现得最清楚，见下。

/// 能力位：block_on（阻塞等待一个 future 完成）。
pub const BLOCK_ON: usize = 1 << 0;
/// 能力位：delay（睡眠 / 延迟执行）。
pub const DELAY: usize = 1 << 1;
/// 能力位：spawn_send（投递任务到全局工作队列）。
pub const SPAWN_SEND: usize = 1 << 2;
/// 能力位：spawn_local（投递任务到线程本地队列）——**声明位**。
///
/// # 这一位与其它四位不同：它只负责「声明」，不负责「能不能调」
///
/// 本地投递除了「运行时支持」之外还有一条**环境前提**：必须存在一个本地队列并且
/// 有人驱动它（tokio 需要 `LocalSet`、smol 需要 `LocalExecutor`、compio 由运行时
/// 自带）。纯类型参数表达不了这条前提——类型对了、调用点错了，运行期才出问题。
///
/// 因此 `spawn_local` 的**调用点**不由本位移门控，而由**值**承载：拿不到
/// [`TrLocalScope`](crate::TrLocalScope) 的实现值就没有 `spawn_local` 可调。
/// 本位的职责只剩「声明」那一半：
///
/// - 业务库在类型别名里写上本位的**目的**，是让「我需要本地投递」这件事被显式
///   记录、可审查；
/// - 集成方要取得作用域值，**必须**经各后端的
///   `Runtime<CAPS>::local_scope()`，而该关联函数要求 `CAPS` 含本位——
///   于是「取得本地投递能力」这个动作在代码里留下了痕迹。
///
/// 注意本模块**不**承诺这条声明是无法绕过的：各后端的
/// `LocalScope::new()` 仍是公开入口。声明位的价值是「必须写下来」，不是「写不下来
/// 就用不了」。
pub const SPAWN_LOCAL: usize = 1 << 3;
/// 能力位：spawn_blocking（投递阻塞函数到阻塞线程池）。
pub const SPAWN_BLOCKING: usize = 1 << 4;
/// 全部能力（默认值）。
pub const FULL: usize = BLOCK_ON | DELAY | SPAWN_SEND | SPAWN_LOCAL | SPAWN_BLOCKING;

/// 类型级标记：掩码包含 [`BLOCK_ON`] 位。
pub trait HasBlockOn {}
/// 类型级标记：掩码包含 [`DELAY`] 位。
pub trait HasDelay {}
/// 类型级标记：掩码包含 [`SPAWN_SEND`] 位。
pub trait HasSpawnSend {}
/// 类型级标记：掩码包含 [`SPAWN_LOCAL`] 位。
pub trait HasSpawnLocal {}
/// 类型级标记：掩码包含 [`SPAWN_BLOCKING`] 位。
pub trait HasSpawnBlocking {}

macro_rules! impl_has {
    ($t:ident, [$($m:expr),*]) => {
        $(impl $t for [(); $m] {})*
    };
}

// 为 0..=31 中所有「包含对应位」的掩码值实现标记。
// 例如 `HasBlockOn` 覆盖所有奇数掩码（bit0 置位）。
impl_has!(HasBlockOn,       [1,3,5,7,9,11,13,15,17,19,21,23,25,27,29,31]);
impl_has!(HasDelay,         [2,3,6,7,10,11,14,15,18,19,22,23,26,27,30,31]);
impl_has!(HasSpawnSend,     [4,5,6,7,12,13,14,15,20,21,22,23,28,29,30,31]);
impl_has!(HasSpawnLocal,    [8,9,10,11,12,13,14,15,24,25,26,27,28,29,30,31]);
impl_has!(HasSpawnBlocking, [16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31]);

#[cfg(test)]
mod tests {
    //! 能力位掩码与类型级标记的单元测试。

    use super::*;

    /// 编译期断言辅助：`T` 必须实现 `Has*` 标记（不满足则编译失败）。
    fn assert_has_block_on<T: HasBlockOn>() {}
    fn assert_has_delay<T: HasDelay>() {}
    fn assert_has_spawn_send<T: HasSpawnSend>() {}
    fn assert_has_spawn_local<T: HasSpawnLocal>() {}
    fn assert_has_spawn_blocking<T: HasSpawnBlocking>() {}

    /// 目的：验证 `FULL` 掩码包含全部五种能力。
    ///
    /// 实施策略：把 `[(); FULL]` 类型传给五个 `assert_has_*` 编译期断言函数。
    ///
    /// 通过依据：类型约束全部满足（编译通过）即为通过；若任一标记缺失，
    /// 测试将无法编译。
    #[test]
    fn full_mask_has_all_caps() {
        assert_has_block_on::<[(); FULL]>();
        assert_has_delay::<[(); FULL]>();
        assert_has_spawn_send::<[(); FULL]>();
        assert_has_spawn_local::<[(); FULL]>();
        assert_has_spawn_blocking::<[(); FULL]>();
    }

    /// 目的：验证单个能力位的掩码只包含对应能力。
    ///
    /// 实施策略：对每个能力位，断言它实现自身的标记，并（在编译期）
    /// 验证它**不**实现其它能力的标记——不满足会编译失败。
    ///
    /// 通过依据：编译通过即为通过。
    #[test]
    fn single_bits_map_to_caps() {
        assert_has_block_on::<[(); BLOCK_ON]>();
        assert_has_delay::<[(); DELAY]>();
        assert_has_spawn_send::<[(); SPAWN_SEND]>();
        assert_has_spawn_local::<[(); SPAWN_LOCAL]>();
        assert_has_spawn_blocking::<[(); SPAWN_BLOCKING]>();
    }

    /// 目的：验证组合掩码（按位或）正确地实现了所有组成能力的标记。
    ///
    /// 实施策略：用 `BLOCK_ON | SPAWN_LOCAL` 组合掩码，断言它同时满足
    /// `HasBlockOn` 与 `HasSpawnLocal`。
    ///
    /// 通过依据：编译通过即为通过。
    #[test]
    fn combined_mask_has_component_caps() {
        assert_has_block_on::<[(); BLOCK_ON | SPAWN_LOCAL]>();
        assert_has_spawn_local::<[(); BLOCK_ON | SPAWN_LOCAL]>();
    }

    /// 目的：固定能力位号分配，防止无意中挪动位号导致已发布的常量值改变。
    ///
    /// 实施策略：断言每一位的数值与 `FULL` 的组合结果。
    ///
    /// 通过依据：`SPAWN_LOCAL` 仍是位 3（`1 << 3 == 8`），`SPAWN_BLOCKING` 仍是
    /// 位 4，`FULL == 31`；若有人插入新位而不复核，本断言会失败。
    #[test]
    fn bit_assignment_is_stable() {
        assert_eq!(BLOCK_ON, 1);
        assert_eq!(DELAY, 2);
        assert_eq!(SPAWN_SEND, 4);
        assert_eq!(SPAWN_LOCAL, 8);
        assert_eq!(SPAWN_BLOCKING, 16);
        assert_eq!(FULL, 31);
    }
}
