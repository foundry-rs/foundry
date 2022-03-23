/// Decoding helpers
pub mod decode;

/// Call trace arena, decoding and formatting
pub mod trace;

/// Debugger data structures
pub mod debug;

/// Forge test execution backends
pub mod executor;
pub use executor::abi;

/// Fuzzing wrapper for executors
pub mod fuzz;

// Re-exports
pub use ethers::types::Address;
pub use hashbrown::HashMap;

use once_cell::sync::Lazy;
pub static CALLER: Lazy<Address> = Lazy::new(Address::random);
