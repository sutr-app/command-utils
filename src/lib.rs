// use pattern feature (may be unstable)
// https://github.com/rust-lang/rust/issues/27721
// (10 years passed, still unstable. So implement by myself for necessary case only)
#![cfg_attr(feature = "unstable", feature(pattern))]

pub mod text;
pub mod util;
