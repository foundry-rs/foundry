//! Shared analysis primitives reused by Solidity lints.
//!
//! - [`primitives`]: HIR probes (`peel_address_wraps`, `is_msg_sender`, ...).
//! - [`facts`]: predicate decomposition and caller-guard recognition.
//! - [`modifier`]: modifier-prefix scanning and param -> caller-arg mapping.
//! - [`interface`]: contract/library function-shape matching.
//!
//! All helpers borrow HIR and never mutate it.

pub mod facts;
pub mod interface;
pub mod modifier;
pub mod primitives;
