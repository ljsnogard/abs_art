test:
    cargo test --workspace
    cargo run -q -p abs_art-demo

# 运行 abs_art-demo 的 tokio 组 cap smoke tests（examples/tokio_demo/）
# 缺省演示组是 compio，因此这里必须显式关掉缺省 features 再选 tokio 组。
demo-tokio:
    for ex in cap_block_on cap_spawn_send cap_spawn_local cap_delay cap_spawn_blocking cap_full cap_zero; do cargo run -q -p abs_art-demo --no-default-features --features demo-tokio --example tokio_$ex || exit 1; done

# 运行 abs_art-demo 的 compio 组 cap smoke tests（examples/compio_demo/）——它是**缺省**组。
# 两组互斥：tokio 组要显式 --no-default-features --features demo-tokio。
demo-compio:
    for ex in cap_block_on cap_spawn_send cap_spawn_local cap_delay cap_spawn_blocking cap_full cap_zero; do cargo run -q -p abs_art-demo --no-default-features --features demo-compio --example compio_$ex || exit 1; done

# 两个演示组全部跑一遍
demo:
    just demo-tokio && just demo-compio

# 单独验证某个后端（bridge 的 backend-* 由集成方在 Cargo.toml 选择）
#
# 注意：必须带 `--no-default-features`。bridge 缺省启用 backend-tokio，若直接
# `--features backend-smol`，feature 并集会是 {tokio, smol}；本版起「链接多个
# 后端」是允许的，但**必须显式声明 default-backend-***，否则会触发 bridge 的
# compile_error（多后端不得靠优先级悄悄选默认）。
test-backend-tokio:
    cargo test -p abs_art-tokio
    cargo check -p abs_art-bridge --no-default-features --features backend-tokio
    just demo-tokio

test-backend-compio:
    cargo test -p abs_art-compio
    cargo check -p abs_art-bridge --no-default-features --features backend-compio
    just demo-compio

test-backend-smol:
    cargo test -p abs_art-smol
    cargo check -p abs_art-bridge --no-default-features --features backend-smol

# 跨后端行为契约矩阵（同一份测试体跑三个真实运行时）
#
# - spawn_local_contract：3 后端 × 4 用例 = 12 格；
# - time_contract：3 后端 × 8 用例 = 24 格（本版新增「计时与时刻来自运行时值」等契约）。
#
# 全部应当通过。spawn_local 的 12 格在最早的**类型级** spawn_local 下曾有两格是红的
# （smol 的「运行时驱动」与「detach 后存活」）；把本地队列交给「作用域值」持有后转绿。
# 本版一度把队列并回**运行时值**，最终又剥回独立作用域（`rt.local_scope()` →
# `scope.spawn_local` / `scope.run_until` / `scope.block_on`）。
# 本配方同时是那次改造的验收标准与回归防线。
smoke:
    cargo test -p abs_art-smoke
