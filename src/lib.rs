pub mod task;

pub enum Runtime {
    Smol,
    Tokio,
}

impl Runtime {
    #[cfg(feature = "runtime-smol")]
    pub const fn current() -> Self {
        Runtime::Smol
    }

    #[cfg(feature = "runtime-tokio")]
    pub const fn current() -> Self {
        Runtime::Tokio
    }
}

#[cfg(
    all(
        feature = "runtime-tokio",
        feature = "runtime-smol"
    )
)]
compile_error!("ONE and ONLY ONE runtime feature can be enabled at the same time");

pub mod x_deps {
    #[cfg(feature = "runtime-smol")]
    pub use smol;

    #[cfg(feature = "runtime-tokio")]
    pub use tokio;
}
