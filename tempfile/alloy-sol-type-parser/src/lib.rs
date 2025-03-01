#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate alloc;

/// Errors.
mod error;
pub use error::{Error, Result};

/// Solidity ident rules.
mod ident;
pub use ident::{is_id_continue, is_id_start, is_valid_identifier, IDENT_REGEX};

/// Root type specifier.
mod root;
pub use root::RootType;

/// Type stem.
mod stem;
pub use stem::TypeStem;

/// Tuple type specifier.
mod tuple;
pub use tuple::TupleSpecifier;

/// Type specifier.
mod type_spec;
pub use type_spec::TypeSpecifier;

/// Parameter specifier.
mod parameter;
pub use parameter::{ParameterSpecifier, Parameters, Storage};

mod state_mutability;
#[cfg(feature = "serde")]
pub use state_mutability::serde_state_mutability_compat;
pub use state_mutability::StateMutability;

// Not public API.
#[doc(hidden)]
pub mod utils;

#[doc(hidden)]
pub mod input;
#[doc(hidden)]
pub use input::{new_input, Input};
