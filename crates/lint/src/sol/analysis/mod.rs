//! Shared analysis primitives reused by Solidity lints.
//!
//! - [`primitives`]: HIR probes (`is_address_type`, `is_require_or_assert`,
//!   `address_call_receiver`, `branch_always_exits`).
//! - [`interface`]: contract/library function-shape matching (`is_elementary`,
//!   `receiver_contract_id`).
//!
//! All helpers borrow HIR and never mutate it.

pub mod interface;
pub mod primitives;
