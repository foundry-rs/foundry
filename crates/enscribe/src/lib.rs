//! enscribe does ENS name setting for contracts.

#[allow(clippy::too_many_arguments)]
mod abi;
pub mod name;

pub use name::set_primary_name;
