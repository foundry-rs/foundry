//! Commonly used constants.

use alloy_eips::Typed2718;
use alloy_network::AnyTxEnvelope;
use alloy_primitives::{Address, B256, Signature, address};
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
pub const ARBITRUM_SENDER: Address = address!("0x00000000000000000000000000000000000a4b05");

/// The system address, the sender of the first transaction in every block:
/// `0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001`
///
/// See also <https://github.com/ethereum-optimism/optimism/blob/65ec61dde94ffa93342728d324fecf474d228e1f/specs/deposits.md#l1-attributes-deposited-transaction>
pub const OPTIMISM_SYSTEM_ADDRESS: Address = address!("0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001");

/// The system address, the sender of the first transaction in every block:
pub const MONAD_SYSTEM_ADDRESS: Address = address!("0x6f49a8F621353f12378d0046E7d7e4b9B249DC9e");

/// Transaction identifier of System transaction types
pub const SYSTEM_TRANSACTION_TYPE: u8 = 126;

/// Default user agent set as the header for requests that don't specify one.
pub const DEFAULT_USER_AGENT: &str = concat!("foundry/", env!("CARGO_PKG_VERSION"));

/// Prefix for auto-generated type bindings using `forge bind-json`.
pub const TYPE_BINDING_PREFIX: &str = "string constant schema_";

/// Returns whether the sender is a known L2 system sender that is the first tx in every block.
///
/// Transactions from these senders usually don't have a any fee information OR set absurdly high fees that exceed the gas limit (See: <https://github.com/foundry-rs/foundry/pull/10608>)
///
/// See: [ARBITRUM_SENDER], [OPTIMISM_SYSTEM_ADDRESS], [MONAD_SYSTEM_ADDRESS] and [Address::ZERO]
pub fn is_known_system_sender(sender: Address) -> bool {
    [ARBITRUM_SENDER, OPTIMISM_SYSTEM_ADDRESS, MONAD_SYSTEM_ADDRESS, Address::ZERO]
        .contains(&sender)
}

pub fn is_impersonated_tx(tx: &AnyTxEnvelope) -> bool {
    if let AnyTxEnvelope::Ethereum(tx) = tx {
        return is_impersonated_sig(tx.signature(), tx.ty());
    }
    false
}

pub fn is_impersonated_sig(sig: &Signature, ty: u8) -> bool {
    let impersonated_sig =
        Signature::from_scalars_and_parity(B256::with_last_byte(1), B256::with_last_byte(1), false);
    if ty != SYSTEM_TRANSACTION_TYPE
        && (sig == &impersonated_sig || sig.r() == impersonated_sig.r())
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_sender() {
        let arb = address!("0x00000000000000000000000000000000000a4b05");
        assert_eq!(arb, ARBITRUM_SENDER);
        let base = address!("0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001");
        assert_eq!(base, OPTIMISM_SYSTEM_ADDRESS);
    }
}
