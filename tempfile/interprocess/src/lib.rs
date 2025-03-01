#![doc = include_str!("../README.md")]
#![cfg_attr(feature = "doc_cfg", feature(doc_cfg))]
// If this was in Cargo.toml, it would cover examples as well
#![warn(
	missing_docs,
	clippy::panic_in_result_fn,
	clippy::missing_assert_message,
	clippy::indexing_slicing,
	clippy::arithmetic_side_effects
)]

mod platform_check;

// TODO(2.3.0) inspect panic points

#[macro_use]
mod macros;

pub mod bound_util;
pub mod error;
pub mod local_socket;
pub mod unnamed_pipe;

/// Platform-specific functionality for various interprocess communication primitives.
///
/// This module houses two modules: `unix` and `windows`, although only one at a time will be
/// visible, depending on which platform the documentation was built on. If you're using
/// [Docs.rs](https://docs.rs/interprocess/latest/interprocess), you can view the documentation for
/// Windows, macOS, Linux and FreeBSD using the Platform menu on the Docs.rs-specific header bar at
/// the top of the page. Docs.rs builds also have the nightly-only `doc_cfg` feature enabled by
/// default, with which everything platform-specific has a badge next to it which specifies the
/// `cfg(...)` conditions for that item to be available.
pub mod os {
	#[cfg(unix)]
	#[cfg_attr(feature = "doc_cfg", doc(cfg(unix)))]
	pub mod unix;
	#[cfg(windows)]
	#[cfg_attr(feature = "doc_cfg", doc(cfg(windows)))]
	pub mod windows;
}

mod try_clone;
pub use try_clone::*;

mod atomic_enum;
mod misc;
pub(crate) use {atomic_enum::*, misc::*};

#[cfg(test)]
#[path = "../tests/index.rs"]
#[allow(
	clippy::unwrap_used,
	clippy::arithmetic_side_effects,
	clippy::indexing_slicing
)]
mod tests;
