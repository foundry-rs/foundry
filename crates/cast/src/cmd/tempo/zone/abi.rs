//! Minimal zone client ABI, mirrored from tempoxyz/zones at a1c15e9f.
//!
//! Source: <https://github.com/tempoxyz/zones/tree/a1c15e9f002a5efd150f56c5076fb35519288396/crates/contracts/src/precompiles>

use alloy_primitives::{Address, address};
use alloy_sol_types::sol;

/// Zone outbox precompile address.
pub(super) const OUTBOX: Address = address!("1c00000000000000000000000000000000000002");

sol! {
    /// Encrypted recipient and memo submitted to the L1 portal.
    struct DepositPayload {
        bytes32 ephemeralPubkeyX;
        uint8 ephemeralPubkeyYParity;
        bytes ciphertext;
        bytes12 nonce;
        bytes16 tag;
    }

    #[sol(rpc)]
    interface IZonePortal {
        function zoneId() external view returns (uint32);
        event WithdrawalProcessed(address indexed to, bytes32 indexed senderTag,
            address token, uint128 amount, bool callbackSuccess);
        function encryptionKeyAtBlock(uint64 tempoBlockNumber)
            external view returns (bytes32 x, uint8 yParity, uint256 keyIndex);
        function deposit(address token, uint128 amount, uint256 keyIndex,
            DepositPayload encrypted, address tempoRefundRecipient)
            external returns (bytes32 newCurrentDepositQueueHash);
    }

    #[sol(rpc)]
    #[allow(clippy::too_many_arguments, reason = "matches the zone protocol ABI")]
    interface IZoneOutbox {
        event WithdrawalRequested(uint64 indexed withdrawalIndex, address indexed sender,
            address token, address to, uint128 amount, uint128 fee, bytes32 memo,
            uint64 gasLimit, uint64 fallbackNonce, bytes data, bytes revealTo);
        function calculateWithdrawalFee(uint64 gasLimit) external view returns (uint128 fee);
        function requestWithdrawal(address token, address to, uint128 amount, bytes32 memo,
            uint64 gasLimit, address fallbackRecipient, bytes callbackData, bytes revealTo)
            external;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::SolCall;

    #[test]
    fn zone_abi_selectors() {
        assert_eq!(IZonePortal::depositCall::SELECTOR, [0x03, 0xdd, 0x6f, 0x34]);
        assert_eq!(IZonePortal::encryptionKeyAtBlockCall::SELECTOR, [0x39, 0xdc, 0x01, 0x5d]);
        assert_eq!(IZoneOutbox::requestWithdrawalCall::SELECTOR, [0xb3, 0xb2, 0x00, 0xaa]);
        assert_eq!(IZoneOutbox::calculateWithdrawalFeeCall::SELECTOR, [0x7b, 0x9c, 0x9a, 0xa4]);
    }
}
