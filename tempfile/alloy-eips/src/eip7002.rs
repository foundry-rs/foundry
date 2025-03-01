//! Contains the system contract and [WithdrawalRequest] types, first introduced in the [Prague hardfork](https://github.com/ethereum/execution-apis/blob/main/src/engine/prague.md).
//!
//! See also [EIP-7002](https://eips.ethereum.org/EIPS/eip-7002): Execution layer triggerable withdrawals

use alloy_primitives::{address, bytes, Address, Bytes, FixedBytes};

/// The caller to be used when calling the EIP-7002 withdrawal requests contract at the end of the
/// block.
pub const SYSTEM_ADDRESS: Address = address!("fffffffffffffffffffffffffffffffffffffffe");

/// The address for the EIP-7002 withdrawal requests contract.
pub const WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS: Address =
    address!("00000961Ef480Eb55e80D19ad83579A64c007002");

/// The code for the EIP-7002 withdrawal requests contract.
pub static WITHDRAWAL_REQUEST_PREDEPLOY_CODE: Bytes = bytes!("   3373fffffffffffffffffffffffffffffffffffffffe1460cb5760115f54807fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff146101f457600182026001905f5b5f82111560685781019083028483029004916001019190604d565b909390049250505036603814608857366101f457346101f4575f5260205ff35b34106101f457600154600101600155600354806003026004013381556001015f35815560010160203590553360601b5f5260385f601437604c5fa0600101600355005b6003546002548082038060101160df575060105b5f5b8181146101835782810160030260040181604c02815460601b8152601401816001015481526020019060020154807fffffffffffffffffffffffffffffffff00000000000000000000000000000000168252906010019060401c908160381c81600701538160301c81600601538160281c81600501538160201c81600401538160181c81600301538160101c81600201538160081c81600101535360010160e1565b910180921461019557906002556101a0565b90505f6002555f6003555b5f54807fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff14156101cd57505f5b6001546002828201116101e25750505f6101e8565b01600290035b5f555f600155604c025ff35b5f5ffd");

/// The [EIP-7685](https://eips.ethereum.org/EIPS/eip-7685) request type for withdrawal requests.
pub const WITHDRAWAL_REQUEST_TYPE: u8 = 0x01;

/// The [EIP-7002](https://eips.ethereum.org/EIPS/eip-7002) defined maximum withdrawal requests per block.
pub const MAX_WITHDRAWAL_REQUESTS_PER_BLOCK: usize = 16;

/// Represents an execution layer triggerable withdrawal request.
///
/// See [EIP-7002](https://eips.ethereum.org/EIPS/eip-7002).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ssz", derive(ssz_derive::Encode, ssz_derive::Decode))]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub struct WithdrawalRequest {
    /// Address of the source of the exit.
    pub source_address: Address,
    /// Validator public key.
    pub validator_pubkey: FixedBytes<48>,
    /// Amount of withdrawn ether in gwei.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::displayfromstr"))]
    pub amount: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;

    #[test]
    #[cfg(feature = "serde")]
    fn test_serde_withdrawal_request() {
        // Sample JSON input representing a withdrawal request
        let json_data = r#"{
            "source_address":"0xAE0E8770147AaA6828a0D6f642504663F10F7d1E",
            "validator_pubkey":"0x8e8d8749f6bc79b78be7cc6e49ff640e608454840c360b344c3a4d9b7428e280e7f40d2271bad65d8cbbfdd43cb8793b",
            "amount":"1"
        }"#;

        // Deserialize the JSON into a WithdrawalRequest struct
        let withdrawal_request: WithdrawalRequest =
            serde_json::from_str(json_data).expect("Failed to deserialize");

        // Verify the deserialized content
        assert_eq!(
            withdrawal_request.source_address,
            address!("AE0E8770147AaA6828a0D6f642504663F10F7d1E")
        );
        assert_eq!(
            withdrawal_request.validator_pubkey,
            FixedBytes::<48>::from(hex!("8e8d8749f6bc79b78be7cc6e49ff640e608454840c360b344c3a4d9b7428e280e7f40d2271bad65d8cbbfdd43cb8793b"))
        );
        assert_eq!(withdrawal_request.amount, 1);

        // Serialize the struct back into JSON
        let serialized_json =
            serde_json::to_string(&withdrawal_request).expect("Failed to serialize");

        // Check if the serialized JSON matches the expected JSON structure
        let expected_json = r#"{"source_address":"0xae0e8770147aaa6828a0d6f642504663f10f7d1e","validator_pubkey":"0x8e8d8749f6bc79b78be7cc6e49ff640e608454840c360b344c3a4d9b7428e280e7f40d2271bad65d8cbbfdd43cb8793b","amount":"1"}"#;
        assert_eq!(serialized_json, expected_json);
    }
}
