#![cfg_attr(not(feature = "std"), no_std)]
#![warn(
    unused,
    future_incompatible,
    nonstandard_style,
    rust_2018_idioms,
    rust_2021_compatibility
)]
#![allow(clippy::op_ref, clippy::suspicious_op_assign_impl)]
#![deny(unsafe_code)]
#![doc = include_str!("../README.md")]

#[macro_use]
extern crate ark_std;

#[macro_use]
extern crate derivative;

#[macro_use]
pub mod biginteger;
pub use self::biginteger::*;

#[macro_use]
pub mod fields;
pub use self::fields::*;

pub(crate) mod bits;
pub use bits::*;

pub(crate) mod const_helpers;

pub use ark_std::UniformRand;

mod to_field_vec;
pub use to_field_vec::ToConstraintField;

pub use num_traits::{One, Zero};

#[doc(hidden)]
pub use ark_ff_asm::*;
#[doc(hidden)]
pub use ark_std::vec;

pub mod prelude {
    pub use crate::biginteger::BigInteger;

    pub use crate::fields::{Field, PrimeField};

    pub use ark_std::UniformRand;

    pub use num_traits::{One, Zero};
}
