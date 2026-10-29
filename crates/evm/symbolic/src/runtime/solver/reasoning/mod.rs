//! Lightweight reasoning passes over normalized solver constraints.

use super::{normalize::ConstraintContext, *};

mod monotonic_product;
mod xor;

pub(super) use monotonic_product::{
    product_monotonic_unsat_normalized, remove_implied_monotonic_constraints,
};
pub(super) use xor::{CheckedXorResult, checked_xor_result};
