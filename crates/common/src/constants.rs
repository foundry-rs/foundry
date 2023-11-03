//! Commonly used constants

use alloy_primitives::{address, Address};
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

/// Alchemy free tier cups: <https://docs.alchemy.com/reference/pricing-plans>
pub const ALCHEMY_FREE_TIER_CUPS: u64 = 330;

/// Logged when an error is indicative that the user is trying to fork from a non-archive node.
pub const NON_ARCHIVE_NODE_WARNING: &str = "\
It looks like you're trying to fork from an older block with a non-archive node which is not \
supported. Please try to change your RPC url to an archive node if the issue persists.";

/// Arbitrum L1 sender address of the first transaction in every block.
/// `0x00000000000000000000000000000000000a4b05`
pub const ARBITRUM_SENDER: Address = address!("00000000000000000000000000000000000a4b05");

/// The system address, the sender of the first transaction in every block:
/// `0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001`
///
/// See also <https://github.com/ethereum-optimism/optimism/blob/65ec61dde94ffa93342728d324fecf474d228e1f/specs/deposits.md#l1-attributes-deposited-transaction>
pub const OPTIMISM_SYSTEM_ADDRESS: Address = address!("deaddeaddeaddeaddeaddeaddeaddeaddead0001");

/// Transaction identifier of System transaction types
pub const SYSTEM_TRANSACTION_TYPE: u64 = 126u64;

/// Returns whether the sender is a known L2 system sender that is the first tx in every block.
///
/// Transactions from these senders usually don't have a any fee information.
///
/// See: [ARBITRUM_SENDER], [OPTIMISM_SYSTEM_ADDRESS]
#[inline]
pub fn is_known_system_sender(sender: Address) -> bool {
    [ARBITRUM_SENDER, OPTIMISM_SYSTEM_ADDRESS].contains(&sender)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_constant_sender() {
        let arb = Address::from_str("0x00000000000000000000000000000000000a4b05").unwrap();
        assert_eq!(arb, ARBITRUM_SENDER);
        let base = Address::from_str("0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001").unwrap();
        assert_eq!(base, OPTIMISM_SYSTEM_ADDRESS);
    }
}
