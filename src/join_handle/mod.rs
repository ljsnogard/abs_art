pub mod handle;

pub use handle::{JoinError, JoinHandle};

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// compio config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime-compio")]
pub(crate) mod spec_compio_;

#[cfg(feature = "runtime-compio")]
pub(crate) use spec_compio_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// smol config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime-smol")]
pub(crate) mod spec_smol_;

#[cfg(feature = "runtime-smol")]
pub(crate) use spec_smol_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// tokio config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime-tokio")]
pub(crate) mod spec_tokio_;

#[cfg(feature = "runtime-tokio")]
pub(crate) use spec_tokio_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// tokio config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(not(any(
    feature = "runtime-compio",
    feature = "runtime-tokio",
    feature = "runtime-smol",
)))]
mod fallback_;

#[cfg(not(any(
    feature = "runtime-compio",
    feature = "runtime-tokio",
    feature = "runtime-smol",
)))]
use fallback_ as runtime_spec_;
