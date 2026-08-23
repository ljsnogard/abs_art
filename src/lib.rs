#![feature(impl_trait_in_assoc_type)]

#![no_std]

#[cfg(test)]
extern crate std;

pub mod runtime;

#[cfg(feature = "join_handle")]
pub mod join_handle;

#[cfg(feature = "delay")]
pub mod delay;

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
        feature = "runtime_compio",
        feature = "runtime_tokio",
        feature = "runtime_smol"
    ),
    all(
        feature = "runtime_compio",
        any(feature = "runtime_tokio", feature = "runtime_smol")
    ),
    all(
        feature = "runtime_tokio",
        any(feature = "runtime_compio", feature = "runtime_smol")
    ),
    all(
        feature = "runtime_smol",
        any(feature = "runtime_compio", feature = "runtime_tokio")
    ),
))]
compile_error!("ONE and ONLY ONE runtime feature can be enabled at the same time");
