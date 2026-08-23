//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// compio config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime-compio")]
pub(crate) mod spec_compio_;

#[allow(unused_imports)]
#[cfg(feature = "runtime-compio")]
pub(crate) use spec_compio_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// smol config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime-smol")]
pub(crate) mod spec_smol_;

#[allow(unused_imports)]
#[cfg(feature = "runtime-smol")]
pub(crate) use spec_smol_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// tokio config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime-tokio")]
pub(crate) mod spec_tokio_;

#[allow(unused_imports)]
#[cfg(feature = "runtime-tokio")]
pub(crate) use spec_tokio_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// unit tests (one file per underlying runtime)
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(all(test, feature = "runtime-compio"))]
mod tests_compio_;

#[cfg(all(test, feature = "runtime-smol"))]
mod tests_smol_;

#[cfg(all(test, feature = "runtime-tokio"))]
mod tests_tokio_;
