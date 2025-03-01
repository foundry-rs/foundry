//! [EIP-7702] constants, helpers, and types.
//!
//! [EIP-7702]: https://eips.ethereum.org/EIPS/eip-7702
#![cfg_attr(not(feature = "std"), no_std)]

#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

mod auth_list;
pub use auth_list::*;

pub mod constants;

mod error;
pub use error::Eip7702Error;

/// Bincode-compatible serde implementations for EIP-7702 types.
///
/// `bincode` crate doesn't work with `#[serde(flatten)]` attribute, but some of the EIP-7702 types
/// require flattenning for RPC compatibility. This module makes so that all fields are
/// non-flattenned.
///
/// Read more: <https://github.com/bincode-org/bincode/issues/167#issuecomment-897629039>
#[cfg(all(feature = "serde", feature = "serde-bincode-compat"))]
pub mod serde_bincode_compat {
    pub use super::auth_list::serde_bincode_compat::*;
}
