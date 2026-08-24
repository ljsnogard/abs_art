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
test-backend-tokio:
    cargo test -p abs_art-tokio
    cargo check -p abs_art-bridge --features backend-tokio
    just demo-tokio

test-backend-compio:
    cargo test -p abs_art-compio
    cargo check -p abs_art-bridge --features backend-compio
    just demo-compio

test-backend-smol:
    cargo test -p abs_art-smol
    cargo check -p abs_art-bridge --features backend-smol
