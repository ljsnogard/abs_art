//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// compio config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime_compio")]
pub(crate) mod spec_compio_;

#[allow(unused_imports)]
#[cfg(feature = "runtime_compio")]
pub(crate) use spec_compio_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// smol config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime_smol")]
pub(crate) mod spec_smol_;

#[allow(unused_imports)]
#[cfg(feature = "runtime_smol")]
pub(crate) use spec_smol_ as runtime_spec_;

//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----
// tokio config
//-- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----

#[cfg(feature = "runtime_tokio")]
pub(crate) mod spec_tokio_;

#[allow(unused_imports)]
#[cfg(feature = "runtime_tokio")]
pub(crate) use spec_tokio_ as runtime_spec_;
