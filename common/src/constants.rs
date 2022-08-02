//! Commonly used constants

use std::time::Duration;

/// The dev chain-id, inherited from hardhat
pub const DEV_CHAIN_ID: u64 = 31337;

/// The first four bytes of the call data for a function call specifies the function to be called.
pub const SELECTOR_LEN: usize = 4;

/// The polling interval to use for local endpoints
pub const LOCAL_HTTP_POLL_INTERVAL: Duration = Duration::from_millis(100);
