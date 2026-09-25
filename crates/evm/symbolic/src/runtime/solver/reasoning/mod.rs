//! Lightweight reasoning passes over normalized solver constraints.

use super::{normalize::ConstraintContext, *};

mod monotonic_product;

pub(super) use monotonic_product::{
    product_monotonic_unsat_normalized, remove_implied_monotonic_constraints,
};

#[cfg(test)]
pub(crate) use monotonic_product::product_monotonic_unsat;
