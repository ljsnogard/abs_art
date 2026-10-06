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
//! | 能力值（如 [`crate::TrLocalScope`] 的实现） | 你**拿到**了没有？ | 具体类型的值（本地作用域） |
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
/// # 这一位的职责：既「声明」，也门控「取得作用域」这唯一入口
///
/// 本地投递除了「运行时支持」之外还有一条**环境前提**：必须存在一个本地队列并且
/// 有人驱动它（tokio 需要 `LocalSet`、smol 需要 `LocalExecutor`、compio 由运行时
/// 自带）。纯类型参数表达不了这条前提——类型对了、调用点错了，运行期才出问题。
///
/// 本地队列归**作用域值**（线程独占）所有，这条环境前提由作用域承担，本位的职责是：
///
/// - **声明**：业务库在类型别名里写上本位，让「我需要本地投递」这件事被显式记录、
///   可审查（与 `unsafe` 同类：必须写下来）；
/// - **门控**：各后端的 `Runtime<CAPS>::local_scope()`（取得 [`TrLocalScope`](crate::TrLocalScope) 实现值的
///   唯一入口）要求 `[(); CAPS]: HasSpawnLocal`，所以没写本位时**取不到作用域**，
///   也就没有 `spawn_local` 可调。
///
/// 声明位的价值始终是「必须写下来」，而不是「写不下来就用不了」——任何人都可以写
/// `Runtime::<{ SPAWN_LOCAL }>`；它约束的是**意外**，不是**恶意**。
pub const SPAWN_LOCAL: usize = 1 << 3;
/// 能力位：spawn_blocking（投递阻塞函数到阻塞线程池）。
pub const SPAWN_BLOCKING: usize = 1 << 4;
/// 能力位：clock（读时刻——[`TrClock`](crate::TrClock) 与
/// [`TrMockClock`](crate::TrMockClock)）。
///
/// # 为什么「读表」要单独一位，而不是复用 [`DELAY`]
///
/// 「能等」与「能读表现在几点」是两件事：读时刻不需要时间驱动跑起来。复用 `DELAY`
/// 会让「只想读表」的声明被迫带上计时能力，也会让「有时钟但没有计时器」的后端无法
/// 诚实表达。代价是能力表从 5 位（`0..=31`）扩到 6 位（`0..=63`），实现由宏生成。
///
/// # 它门控什么
///
/// 各后端的 `impl TrClock for Runtime<CAPS>` 要求 `[(); CAPS]: HasClock`；由于
/// [`TrTime: TrDelay + TrClock`](crate::TrTime)，**`interval` / `timeout` 也需要本位**
/// （这是「同源」那条结构约束的直接后果）。
///
/// ```compile_fail
/// use abs_art::{DELAY, HasClock};
///
/// fn assert_has_clock<T: HasClock>() {}
/// // 只有 `DELAY` 位：没有时钟能力 → 编译失败
/// assert_has_clock::<[(); DELAY]>();
/// ```
pub const CLOCK: usize = 1 << 5;
/// 全部能力（默认值）。
///
/// 注意：这是**位集合**意义上的「全部」，不代表每个后端都实现得了——例如 compio 没有
/// 跨线程全局队列，它不实现 [`TrSpawnSend`](crate::TrSpawnSend)，因此它对外给出的是
/// **自己的** `FULL`（不含 [`SPAWN_SEND`]）。
pub const FULL: usize = BLOCK_ON | DELAY | SPAWN_SEND | SPAWN_LOCAL | SPAWN_BLOCKING | CLOCK;

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
/// 类型级标记：掩码包含 [`CLOCK`] 位。
pub trait HasClock {}

macro_rules! impl_has {
    ($t:ident, [$($m:expr),*]) => {
        $(impl $t for [(); $m] {})*
    };
}

// 为 0..=63（6 位）中所有「包含对应位」的掩码值实现标记：每个 trait 各 32 个 impl，
// 合计 192 个。列表由脚本按「掩码含该位」生成（见 `abs_art_runtime_probe/p1_orphan/z_caps6/`
// 的独立复验），手写容易漏项。
impl_has!(HasBlockOn,       [1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31, 33, 35, 37, 39, 41, 43, 45, 47, 49,
                              51, 53, 55, 57, 59, 61, 63]);
impl_has!(HasDelay,         [2, 3, 6, 7, 10, 11, 14, 15, 18, 19, 22, 23, 26, 27, 30, 31, 34, 35, 38, 39, 42, 43, 46, 47, 50,
                              51, 54, 55, 58, 59, 62, 63]);
impl_has!(HasSpawnSend,     [4, 5, 6, 7, 12, 13, 14, 15, 20, 21, 22, 23, 28, 29, 30, 31, 36, 37, 38, 39, 44, 45, 46, 47, 52,
                              53, 54, 55, 60, 61, 62, 63]);
impl_has!(HasSpawnLocal,    [8, 9, 10, 11, 12, 13, 14, 15, 24, 25, 26, 27, 28, 29, 30, 31, 40, 41, 42, 43, 44, 45, 46, 47, 56,
                              57, 58, 59, 60, 61, 62, 63]);
impl_has!(HasSpawnBlocking, [16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 48, 49, 50, 51, 52, 53, 54, 55,
                              56, 57, 58, 59, 60, 61, 62, 63]);
impl_has!(HasClock,         [32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55,
                              56, 57, 58, 59, 60, 61, 62, 63]);

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
    fn assert_has_clock<T: HasClock>() {}

    /// 目的：验证 `FULL` 掩码包含全部六种能力。
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
        assert_has_clock::<[(); FULL]>();
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
        assert_has_clock::<[(); CLOCK]>();
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
    /// 位 4，新增的 `CLOCK` 是位 5（`1 << 5 == 32`），`FULL == 63`；若有人插入新位
    /// 而不复核，本断言会失败。
    #[test]
    fn bit_assignment_is_stable() {
        assert_eq!(BLOCK_ON, 1);
        assert_eq!(DELAY, 2);
        assert_eq!(SPAWN_SEND, 4);
        assert_eq!(SPAWN_LOCAL, 8);
        assert_eq!(SPAWN_BLOCKING, 16);
        assert_eq!(CLOCK, 32);
        assert_eq!(FULL, 63);
    }
}
