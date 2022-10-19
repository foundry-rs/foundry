//! Commonly used constants

use std::time::Duration;

/// The dev chain-id, inherited from hardhat
pub const DEV_CHAIN_ID: u64 = 31337;

/// The first four bytes of the call data for a function call specifies the function to be called.
pub const SELECTOR_LEN: usize = 4;

/// Maximum size in bytes (0x6000) that a contract can have.
pub const CONTRACT_MAX_SIZE: usize = 24576;

/// Default request timeout for http requests
///
/// Note: this is only used so that connections, that are discarded on the server side won't stay
/// open forever. We assume some nodes may have some backoff baked into them and will delay some
/// responses. This timeout should be a reasonable amount of time to wait for a request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

/// Alchemy free tier cups <https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups>
pub const ALCHEMY_FREE_TIER_CUPS: u64 = 330;
