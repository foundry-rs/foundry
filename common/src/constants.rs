//! Commonly used constants

use std::time::Duration;

/// The dev chain-id, inherited from hardhat
pub const DEV_CHAIN_ID: u64 = 31337;

/// The first four bytes of the call data for a function call specifies the function to be called.
pub const SELECTOR_LEN: usize = 4;

/// Maximum size in bytes (0x6000) that a contract can have.
pub const CONTRACT_MAX_SIZE: usize = 24576;

/// Default request timeout for http requests
pub const REQUEST_TIMEOUT: Duration = Duration::from_millis(30_000);
