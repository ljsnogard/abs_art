//! 跨后端共享的探测体：**同一份代码**分别跑在 tokio / compio / smol 上。
//!
//! 这里的三个函数都泛型于 `R: TrLocalScope`，接收的是**运行时值**（`&R`）而不是
//! 运行时类型参数、也不是独立的作用域对象——这正是 v0.4「值化」的核心：本地队列
//! 从「隐式环境」变成「运行时值自己持有的东西」，后端差异全部被那个值吃掉，
//! 业务侧一份代码三个后端通用。
//!
//! 与 v0.3 的差别只在**调用形状**上：那时能力是无 `self` 的关联函数、本地队列挂在
//! 另一个 `LocalScope` 值上（`<Rt as TrSpawnSend>::spawn(f)` + `scope.spawn_local(f)`）；
//! 现在两者都并到了同一个值上（`rt.spawn(f)` + `rt.spawn_local(f)`）。
//! 三条契约的**语义与判定标准一字未改**。
//!
//! 三个探测点从弱到强：
//!
//! - [`probe_a_handle_driven`]：宿主 spawn 后立即 await `JoinHandle`；
//! - [`probe_b_runtime_driven`]：宿主**不 poll 任何句柄**，先等循环自己回报；
//! - [`probe_c_detach_survives`]：`detach()` 之后循环仍须被调度。

use core::array::from_fn;
use std::cell::RefCell;
use std::rc::Rc;

use abs_art::{TrJoinHandle, TrLocalScope};
use async_channel::unbounded;

/// 冒烟测试同时投放的循环任务个数。
///
/// 取 3 而不是 1：`smux_v1` 的每条连接都同时跑读、写两个循环，单个任务的用例
/// 测不出「多个本地任务各自持有 `!Send` 状态、必须被同一个本地队列推进」这件事。
pub const LOOP_COUNT: usize = 3;

/// 每个循环任务在收到退出消息之前会消费的消息条数。
pub const MSG_PER_LOOP: usize = 4;

/// 投递给循环任务的消息。
///
/// 形状与 `smux_v1` 的循环一致：正常消息若干条，最后一条是「退出循环」的消息。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Msg {
    /// 累加一个值（对应一次正常的收包 / 事件）。
    Add(u32),
    /// 退出循环（对应关闭通知 / 取消令牌生效前的最后一条消息）。
    Stop,
}

/// 循环任务的返回值：`(循环编号, 该循环收到的消息值之和)`。
///
/// 带上编号是为了让「结果经 `JoinHandle` 交回上层」这一步可以被逐条核对——
/// 若句柄把结果张冠李戴，编号比对会立刻发现。
pub type LoopResult = (usize, u32);

/// 第 `idx` 个循环应当收到的消息值序列。
///
/// 每个循环收到的序列**互不相同**（起点为 `idx + 1`），因此结果一旦串台就能
/// 被检出。
pub fn expected_values(idx: usize) -> [u32; MSG_PER_LOOP] {
    let base = idx as u32 + 1;
    from_fn(|k| base + k as u32)
}

/// 第 `idx` 个循环的预期结果之和。
pub fn expected_sum(idx: usize) -> u32 {
    expected_values(idx).iter().sum()
}

/// 探测点 A：句柄驱动。
///
/// 宿主经运行时值 `spawn_local` 之后**立即 await `JoinHandle`**，由句柄把结果交回
/// 上层。这是最弱的一条用法，但它同时验证了「投递点」与「驱动点」都在同一个值上：
/// 句柄能拿到结果，说明这个运行时值确实在驱动它自己的本地队列。
///
/// # Errors
///
/// 句柄返回 join 错误，或任务里记录到的本地状态与句柄结果不一致。
pub async fn probe_a_handle_driven<R>(rt: &R) -> Result<u32, String>
where
    R: TrLocalScope,
{
    // 捕获 Rc 把 future 钉成 !Send：确保走的确实是本地队列，
    // 而不是一个「恰好也能用全局 spawn 跑」的 Send 任务。
    let marker = Rc::new(RefCell::new(0u32));
    let task_local = marker.clone();

    let handle = rt.spawn_local(async move {
        *task_local.borrow_mut() += 42;
        42u32
    });

    let joined = handle
        .await
        .map_err(|e| format!("JoinHandle 返回错误：{e}"))?;

    let local = *marker.borrow();
    if local != joined {
        return Err(format!("任务内本地状态 {local} 与句柄结果 {joined} 不一致"));
    }
    Ok(joined)
}

/// 探测点 B：**运行时值驱动**——宿主不 poll 任何 `JoinHandle`。
///
/// 流程严格按 `smux_v1` 所需语义编排：
///
/// 1. 经运行时值投递 [`LOOP_COUNT`] 个「消费消息死循环」任务，每个任务捕获一个
///    `Rc`（`!Send`）并独占一个消息通道；
/// 2. 宿主把正常消息与**最后一条退出消息**投递进去；
/// 3. 宿主先等各循环**自己**发出的完成回执——此刻宿主还没有 poll 过任何
///    `JoinHandle`，因此循环能推进只能来自「这个运行时值在驱动本地队列」；
/// 4. 之后再逐个 await `JoinHandle`，核对每个循环的结果。
///
/// 第 3 步是本探测点的全部价值所在：若本地队列只能靠 poll 句柄来推进，
/// 宿主会在这里被永久阻塞。
///
/// # Errors
///
/// 投递失败、循环未在期限内回报、句柄返回 join 错误，或结果与预期不符。
pub async fn probe_b_runtime_driven<R>(rt: &R) -> Result<Vec<LoopResult>, String>
where
    R: TrLocalScope,
{
    let (done_tx, done_rx) = unbounded::<usize>();

    // 所有循环共同维护的「已消费消息条数」计数，同时也是把 future 钉成 !Send 的 Rc
    let consumed = Rc::new(RefCell::new(0usize));

    let mut msg_txs = Vec::with_capacity(LOOP_COUNT);
    let mut handles = Vec::with_capacity(LOOP_COUNT);

    for idx in 0..LOOP_COUNT {
        let (msg_tx, msg_rx) = unbounded::<Msg>();
        msg_txs.push(msg_tx);

        let done = done_tx.clone();
        let counter = consumed.clone();

        handles.push(rt.spawn_local(async move {
            let mut sum = 0u32;
            // 刻意写成显式的 `loop` + `match`，而不是 clippy 建议的
            // `while let Ok(..) = ..`：本用例测的就是「最后一个退出消息让循环跳出」
            // 这条契约，退出分支必须写在代码表面上。
            #[allow(clippy::while_let_loop)]
            loop {
                match msg_rx.recv().await {
                    Ok(Msg::Add(v)) => {
                        sum += v;
                        *counter.borrow_mut() += 1;
                    }
                    // 收到退出消息（或发送端消失）才跳出循环
                    Ok(Msg::Stop) | Err(_) => break,
                }
            }
            // 先向宿主回报「我已退出」，再把结果留给 JoinHandle
            let _ = done.send(idx).await;
            (idx, sum)
        }));
    }

    // 循环任务持有 done_tx 的克隆；这里丢掉宿主持有的那一份，
    // 使得「所有循环都跑完」时通道自然关闭。
    drop(done_tx);

    for (idx, tx) in msg_txs.iter().enumerate() {
        for value in expected_values(idx) {
            tx.send(Msg::Add(value))
                .await
                .map_err(|e| format!("向第 {idx} 个循环投递消息失败：{e}"))?;
        }
        tx.send(Msg::Stop)
            .await
            .map_err(|e| format!("向第 {idx} 个循环投递退出消息失败：{e}"))?;
    }

    // 关键一步：先等循环自己回报完成。此刻没有任何 JoinHandle 被 poll，
    // 循环的推进只能来自作用域对本地队列的驱动。
    for _ in 0..LOOP_COUNT {
        done_rx
            .recv()
            .await
            .map_err(|e| format!("循环任务未回报完成，宿主被阻塞（本地队列没有被驱动）：{e}"))?;
    }

    // 再经 JoinHandle 把结果收回上层
    let mut results = Vec::with_capacity(LOOP_COUNT);
    for handle in handles {
        results.push(
            handle
                .await
                .map_err(|e| format!("JoinHandle 返回错误：{e}"))?,
        );
    }
    results.sort_unstable_by_key(|(idx, _)| *idx);

    // 逐条核对：编号、每个循环的求和、以及「所有消息都被消费过」
    for (idx, sum) in &results {
        let want = expected_sum(*idx);
        if *sum != want {
            return Err(format!(
                "第 {idx} 个循环经句柄交回 {sum}，预期 {want}（消息未被完整消费或结果串台）"
            ));
        }
    }

    let consumed = *consumed.borrow();
    let want_consumed = LOOP_COUNT * MSG_PER_LOOP;
    if consumed != want_consumed {
        return Err(format!(
            "共消费 {consumed} 条消息，预期 {want_consumed} 条（退出消息抢在正常消息之前被处理）"
        ));
    }

    Ok(results)
}

/// 探测点 C：`detach()` 之后循环必须继续被调度。
///
/// 这是 `smux_v1` 的实际用法：`rt.spawn_local(read_fut).detach()`——投递之后
/// **立即**脱手，此后不存句柄、不 poll 句柄，循环完全靠运行时值推进，直到自己收到
/// 退出消息（`smux_v1` 用取消令牌 + 消息收尾）。
///
/// 本探测点是「本地队列归运行时值所有」这条设计的直接试金石：句柄被消费后任务还能
/// 推进，只可能是因为队列的持有者是运行时值而不是句柄。
///
/// # Errors
///
/// 循环未回报（被取消或从未被调度），或回报的求和与预期不符。
pub async fn probe_c_detach_survives<R>(rt: &R) -> Result<u32, String>
where
    R: TrLocalScope,
{
    let (msg_tx, msg_rx) = unbounded::<Msg>();
    let (done_tx, done_rx) = unbounded::<u32>();

    let marker = Rc::new(RefCell::new(0u32));
    let task_local = marker.clone();

    let handle = rt.spawn_local(async move {
        let mut sum = 0u32;
        #[allow(clippy::while_let_loop)]
        loop {
            match msg_rx.recv().await {
                Ok(Msg::Add(v)) => sum += v,
                Ok(Msg::Stop) | Err(_) => break,
            }
        }
        *task_local.borrow_mut() = sum;
        let _ = done_tx.send(sum).await;
    });

    // 立即脱手：此后宿主再也没有句柄可 poll
    handle.detach();

    for value in expected_values(0) {
        msg_tx.send(Msg::Add(value)).await.map_err(|e| {
            format!("投递消息失败：循环的接收端已被丢弃——说明 detach 把任务一起取消了（{e}）")
        })?;
    }
    msg_tx.send(Msg::Stop).await.map_err(|e| {
        format!("投递退出消息失败：循环的接收端已被丢弃——说明 detach 把任务一起取消了（{e}）")
    })?;

    let sum = done_rx
        .recv()
        .await
        .map_err(|e| format!("detach 之后循环没有推进（任务被取消或从未被调度）：{e}"))?;

    let want = expected_sum(0);
    if sum != want {
        return Err(format!("detach 后循环回报 {sum}，预期 {want}"));
    }
    Ok(sum)
}
