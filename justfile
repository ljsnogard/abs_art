test:
    cargo test --workspace
    cargo run -q -p abs_art-demo

# 单独验证某个后端（bridge 的 backend-* 由集成方在 Cargo.toml 选择）
test-backend tokio:
    cargo test -p abs_art-tokio
    cargo check -p abs_art-bridge --features backend-tokio

test-backend compio:
    cargo test -p abs_art-compio
    cargo check -p abs_art-bridge --features backend-compio

test-backend smol:
    cargo test -p abs_art-smol
    cargo check -p abs_art-bridge --features backend-smol
