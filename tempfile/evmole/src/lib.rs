//! Analyzes EVM bytecode to extract contract information, even for unverified contracts.
//!
//! The library can extract function selectors, function arguments, state mutability, and storage layout.
//!
//! Use the [`contract_info()`] function with its builder pattern to analyze contracts. See its documentation for usage examples.
//!
//! Accuracy and speed comparison with other tools, as well as Python and JavaScript implementations, are available on [GitHub](https://github.com/cdump/evmole/tree/master#benchmark)

#[allow(deprecated)]
pub use arguments::function_arguments;

#[allow(deprecated)]
pub use arguments::function_arguments_alloy;

#[allow(deprecated)]
pub use selectors::function_selectors;

#[allow(deprecated)]
pub use state_mutability::function_state_mutability;

pub use contract_info::contract_info;
pub use contract_info::{Contract, Function, ContractInfoArgs};
pub use storage::StorageRecord;

mod arguments;
mod evm;
mod selectors;
mod state_mutability;
mod utils;
mod storage;
mod contract_info;

/// A 4-byte function selector
pub type Selector = [u8; 4];

/// A 32-byte storage slot identifier in EVM storage.
pub type Slot = [u8; 32];

/// Function's state mutability
pub type StateMutability = alloy_dyn_abi::parser::StateMutability;

#[cfg(feature = "python")]
mod interface_py;

#[cfg(feature = "javascript")]
mod interface_js;
