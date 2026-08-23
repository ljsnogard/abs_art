#![feature(impl_trait_in_assoc_type)]

#![no_std]

#[cfg(test)]
extern crate std;

pub mod runtime;

#[cfg(feature = "join_handle")]
pub mod join_handle;

#[cfg(feature = "block_on")]
pub mod block_on;

#[cfg(feature = "join_handle")]
pub use join_handle::{JoinHandle, JoinError};

pub use runtime::Runtime;

#[cfg(feature = "spawn_send")]
pub use runtime::TrSpawnSend;

#[cfg(feature = "spawn_blocking")]
pub use runtime::TrSpawnBlocking;

#[cfg(feature = "spawn_local")]
pub use runtime::TrSpawnLocal;

#[cfg(any(
    all(
        feature = "runtime-compio",
        feature = "runtime-tokio",
        feature = "runtime-smol"
    ),
    all(
        feature = "runtime-compio",
        any(feature = "runtime-tokio", feature = "runtime-smol")
    ),
    all(
        feature = "runtime-tokio",
        any(feature = "runtime-compio", feature = "runtime-smol")
    ),
    all(
        feature = "runtime-smol",
        any(feature = "runtime-compio", feature = "runtime-tokio")
    ),
))]
compile_error!("ONE and ONLY ONE runtime feature can be enabled at the same time");
