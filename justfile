test:
    cargo test --workspace
    cargo run -q -p abs_art-demo

# 运行 abs_art-demo 的 tokio 组 cap smoke tests（examples/tokio_demo/）
demo-tokio:
    for ex in cap_block_on cap_spawn_send cap_spawn_local cap_delay cap_spawn_blocking cap_full cap_zero; do cargo run -q -p abs_art-demo --example tokio_$ex || exit 1; done

# 运行 abs_art-demo 的 compio 组 cap smoke tests（examples/compio_demo/）
# compio 组与 tokio 组互斥：需要 --no-default-features --features demo-compio
demo-compio:
    for ex in cap_block_on cap_spawn_send cap_spawn_local cap_delay cap_spawn_blocking cap_full cap_zero; do cargo run -q -p abs_art-demo --no-default-features --features demo-compio --example compio_$ex || exit 1; done

# 两个演示组全部跑一遍
demo:
    just demo-tokio && just demo-compio

# 单独验证某个后端（bridge 的 backend-* 由集成方在 Cargo.toml 选择）
#
# 注意：必须带 `--no-default-features`。bridge 的缺省后端是 tokio，若直接
# `--features backend-smol`，feature 并集会是 {tokio, smol}，触发「后端只能启用
# 一个」的 compile_error。
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

# 跨后端 spawn_local 行为契约矩阵（3 个后端 × 3 个用例，同一份测试体）
#
# 注意：在这轮「让 spawn_local 在三个后端上行为一致」的改造落地之前，
# smol 的 B（运行时驱动）与 C（detach 后存活）两格是**预期失败**——
# 这正是本冒烟测试要固定的缺口，见 abs_art-smoke 的 crate 文档。
smoke:
    cargo test -p abs_art-smoke
