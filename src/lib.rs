pub mod task;

pub enum Runtime {
    AsyncStd,
    Smol,
    Tokio,
}

impl Runtime {
    #[cfg(feature = "runtime-async-std")]
    pub const fn current() -> Self {
        Runtime::AsyncStd
    }

    #[cfg(feature = "runtime-smol")]
    pub const fn current() -> Self {
        Runtime::Smol
    }

    #[cfg(feature = "runtime-tokio")]
    pub const fn current() -> Self {
        Runtime::Tokio
    }
}

#[cfg(any(
    all(
        feature = "runtime-async-std",
        feature = "runtime-tokio",
        feature = "runtime-smol"
    ),
    all(
        feature = "runtime-async-std",
        any(feature = "runtime-tokio", feature = "runtime-smol")
    ),
    all(
        feature = "runtime-tokio",
        any(feature = "runtime-async-std", feature = "runtime-smol")
    ),
    all(
        feature = "runtime-smol",
        any(feature = "runtime-async-std", feature = "runtime-tokio")
    ),
))]
compile_error!("ONE and ONLY ONE runtime feature can be enabled at the same time");

pub mod x_deps {
    #[cfg(feature = "runtime-async-std")]
    pub use async_std;

    #[cfg(feature = "runtime-smol")]
    pub use smol;

    #[cfg(feature = "runtime-tokio")]
    pub use tokio;
}
