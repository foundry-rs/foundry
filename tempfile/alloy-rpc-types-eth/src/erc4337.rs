use crate::{Log, TransactionReceipt};
use alloc::vec::Vec;
use alloy_consensus::conditional::BlockConditionalAttributes;
use alloy_primitives::{
    map::{AddressHashMap, HashMap},
    Address, BlockNumber, Bytes, B256, U256,
};

/// Alias for backwards compat
#[deprecated(since = "0.8.4", note = "use `TransactionConditional` instead")]
pub type ConditionalOptions = TransactionConditional;

/// Options for conditional raw transaction submissions.
///
/// TransactionConditional represents the preconditions that determine the inclusion of the
/// transaction, enforced out-of-protocol by the sequencer.
///
/// See also <https://github.com/ethereum-optimism/op-geth/blob/928070c7fc097362ed2d40a4f72889ba91544931/core/types/transaction_conditional.go#L74-L76>.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct TransactionConditional {
    /// A map of account addresses to their expected storage states.
    /// Each account can have a specified storage root or explicit slot-value pairs.
    #[cfg_attr(feature = "serde", serde(default))]
    pub known_accounts: AddressHashMap<AccountStorage>,
    /// The minimal block number at which the transaction can be included.
    /// `None` indicates no minimum block number constraint.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            with = "alloy_serde::quantity::opt",
            skip_serializing_if = "Option::is_none"
        )
    )]
    pub block_number_min: Option<BlockNumber>,
    /// The maximal block number at which the transaction can be included.
    /// `None` indicates no maximum block number constraint.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            with = "alloy_serde::quantity::opt",
            skip_serializing_if = "Option::is_none"
        )
    )]
    pub block_number_max: Option<BlockNumber>,
    /// The minimal timestamp at which the transaction can be included.
    /// `None` indicates no minimum timestamp constraint.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            with = "alloy_serde::quantity::opt",
            skip_serializing_if = "Option::is_none"
        )
    )]
    pub timestamp_min: Option<u64>,
    /// The maximal timestamp at which the transaction can be included.
    /// `None` indicates no maximum timestamp constraint.
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            with = "alloy_serde::quantity::opt",
            skip_serializing_if = "Option::is_none"
        )
    )]
    pub timestamp_max: Option<u64>,
}

impl TransactionConditional {
    /// Returns true if any configured block parameter (`timestamp_max`, `block_number_max`) are
    /// exceeded by the given block parameter.
    ///
    /// E.g. the block parameter's timestamp is higher than the configured `block_number_max`
    pub const fn has_exceeded_block_attributes(&self, block: &BlockConditionalAttributes) -> bool {
        self.has_exceeded_block_number(block.number) || self.has_exceeded_timestamp(block.timestamp)
    }

    /// Returns true if the configured max block number is lower or equal to the given
    /// `block_number`
    pub const fn has_exceeded_block_number(&self, block_number: BlockNumber) -> bool {
        let Some(max_num) = self.block_number_max else { return false };
        block_number >= max_num
    }

    /// Returns true if the configured max timestamp is lower or equal to the given `timestamp`
    pub const fn has_exceeded_timestamp(&self, timestamp: u64) -> bool {
        let Some(max_timestamp) = self.timestamp_max else { return false };
        timestamp >= max_timestamp
    }

    /// Returns `true` if the transaction matches the given block attributes.
    pub const fn matches_block_attributes(&self, block: &BlockConditionalAttributes) -> bool {
        self.matches_block_number(block.number) && self.matches_timestamp(block.timestamp)
    }

    /// Returns `true` if the transaction matches the given block number.
    pub const fn matches_block_number(&self, block_number: BlockNumber) -> bool {
        if let Some(min) = self.block_number_min {
            if block_number < min {
                return false;
            }
        }
        if let Some(max) = self.block_number_max {
            if block_number > max {
                return false;
            }
        }
        true
    }

    /// Returns `true` if the transaction matches the given timestamp.
    pub const fn matches_timestamp(&self, timestamp: u64) -> bool {
        if let Some(min) = self.timestamp_min {
            if timestamp < min {
                return false;
            }
        }
        if let Some(max) = self.timestamp_max {
            if timestamp > max {
                return false;
            }
        }
        true
    }

    /// Computes the aggregate cost of the preconditions; total number of storage lookups required
    pub fn cost(&self) -> u64 {
        let mut cost = 0;
        for account in self.known_accounts.values() {
            // default cost to handle empty accounts
            cost += 1;
            match account {
                AccountStorage::RootHash(_) => {
                    cost += 1;
                }
                AccountStorage::Slots(slots) => {
                    cost += slots.len() as u64;
                }
            }
        }

        if self.block_number_min.is_some() || self.block_number_max.is_some() {
            cost += 1;
        }
        if self.timestamp_min.is_some() || self.timestamp_max.is_some() {
            cost += 1;
        }

        cost
    }
}

/// Represents the expected state of an account for a transaction to be conditionally accepted.
///
/// Allows for a user to express their preference of a known prestate at a particular account. Only
/// one of the storage root or storage slots is allowed to be set. If the storage root is set, then
/// the user prefers their transaction to only be included in a block if the account's storage root
/// matches. If the storage slots are set, then the user prefers their transaction to only be
/// included if the particular storage slot values from state match.
#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum AccountStorage {
    /// Expected storage root hash of the account.
    RootHash(B256),
    /// Explicit storage slots and their expected values.
    Slots(HashMap<U256, B256>),
}

impl AccountStorage {
    /// Returns `true` if the account storage is a root hash.
    pub const fn is_root(&self) -> bool {
        matches!(self, Self::RootHash(_))
    }

    /// Returns the slot values if the account storage is a slot map.
    pub const fn as_slots(&self) -> Option<&HashMap<U256, B256>> {
        match self {
            Self::Slots(slots) => Some(slots),
            _ => None,
        }
    }
}

/// [`UserOperation`] in the spec: Entry Point V0.6
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct UserOperation {
    /// The address of the smart contract account
    pub sender: Address,
    /// Anti-replay protection; also used as the salt for first-time account creation
    pub nonce: U256,
    /// Code used to deploy the account if not yet on-chain
    pub init_code: Bytes,
    /// Data that's passed to the sender for execution
    pub call_data: Bytes,
    /// Gas limit for execution phase
    pub call_gas_limit: U256,
    /// Gas limit for verification phase
    pub verification_gas_limit: U256,
    /// Gas to compensate the bundler
    pub pre_verification_gas: U256,
    /// Maximum fee per gas
    pub max_fee_per_gas: U256,
    /// Maximum priority fee per gas
    pub max_priority_fee_per_gas: U256,
    /// Paymaster Contract address and any extra data required for verification and execution
    /// (empty for self-sponsored transaction)
    pub paymaster_and_data: Bytes,
    /// Used to validate a UserOperation along with the nonce during verification
    pub signature: Bytes,
}

/// [`PackedUserOperation`] in the spec: Entry Point V0.7
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct PackedUserOperation {
    /// The account making the operation.
    pub sender: Address,
    /// Prevents message replay attacks and serves as a randomizing element for initial user
    /// registration.
    pub nonce: U256,
    /// Deployer contract address: Required exclusively for deploying new accounts that don't yet
    /// exist on the blockchain.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub factory: Option<Address>,
    /// Factory data for the account creation process, applicable only when using a deployer
    /// contract.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub factory_data: Option<Bytes>,
    /// The call data.
    pub call_data: Bytes,
    /// The gas limit for the call.
    pub call_gas_limit: U256,
    /// The gas limit for the verification.
    pub verification_gas_limit: U256,
    /// Prepaid gas fee: Covers the bundler's costs for initial transaction validation and data
    /// transmission.
    pub pre_verification_gas: U256,
    /// The maximum fee per gas.
    pub max_fee_per_gas: U256,
    /// The maximum priority fee per gas.
    pub max_priority_fee_per_gas: U256,
    /// Paymaster contract address: Needed if a third party is covering transaction costs; left
    /// blank for self-funded accounts.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub paymaster: Option<Address>,
    /// The gas limit for the paymaster verification.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub paymaster_verification_gas_limit: Option<U256>,
    /// The gas limit for the paymaster post-operation.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub paymaster_post_op_gas_limit: Option<U256>,
    /// The paymaster data.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub paymaster_data: Option<Bytes>,
    /// The signature of the transaction.
    pub signature: Bytes,
}

/// Send User Operation
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SendUserOperation {
    /// User Operation
    EntryPointV06(UserOperation),
    /// Packed User Operation
    EntryPointV07(PackedUserOperation),
}

/// Response to sending a user operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SendUserOperationResponse {
    /// The hash of the user operation.
    pub user_op_hash: Bytes,
}

/// Represents the receipt of a user operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct UserOperationReceipt {
    /// The hash of the user operation.
    pub user_op_hash: Bytes,
    /// The entry point address for the user operation.
    pub entry_point: Address,
    /// The address of the sender of the user operation.
    pub sender: Address,
    /// The nonce of the user operation.
    pub nonce: U256,
    /// The address of the paymaster, if any.
    pub paymaster: Address,
    /// The actual gas cost incurred by the user operation.
    pub actual_gas_cost: U256,
    /// The actual gas used by the user operation.
    pub actual_gas_used: U256,
    /// Indicates whether the user operation was successful.
    pub success: bool,
    /// The reason for failure, if any.
    pub reason: Bytes,
    /// The logs generated by the user operation.
    pub logs: Vec<Log>,
    /// The transaction receipt of the user operation.
    pub receipt: TransactionReceipt,
}

/// Represents the gas estimation for a user operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct UserOperationGasEstimation {
    /// The gas limit for the pre-verification.
    pub pre_verification_gas: U256,
    /// The gas limit for the verification.
    pub verification_gas: U256,
    /// The gas limit for the paymaster verification.
    pub paymaster_verification_gas: U256,
    /// The gas limit for the call.
    pub call_gas_limit: U256,
}
