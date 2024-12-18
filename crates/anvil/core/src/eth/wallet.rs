use alloy_primitives::{map::HashMap, Address, ChainId, U64};
use serde::{Deserialize, Serialize};

/// The capability to perform [EIP-7702][eip-7702] delegations, sponsored by the sequencer.
///
/// The sequencer will only perform delegations, and act on behalf of delegated accounts, if the
/// account delegates to one of the addresses specified within this capability.
///
/// [eip-7702]: https://eips.ethereum.org/EIPS/eip-7702
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize, Default)]
pub struct DelegationCapability {
    /// A list of valid delegation contracts.
    pub addresses: Vec<Address>,
}

/// Wallet capabilities for a specific chain.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize, Default)]
pub struct Capabilities {
    /// The capability to delegate.
    pub delegation: DelegationCapability,
}

/// A map of wallet capabilities per chain ID.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize, Default)]
pub struct WalletCapabilities(HashMap<U64, Capabilities>);

impl WalletCapabilities {
    /// Get the capabilities of the wallet API for the specified chain ID.
    pub fn get(&self, chain_id: ChainId) -> Option<&Capabilities> {
        self.0.get(&U64::from(chain_id))
    }

    pub fn insert(&mut self, chain_id: ChainId, capabilities: Capabilities) {
        self.0.insert(U64::from(chain_id), capabilities);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// The transaction value is not 0.
    ///
    /// The value should be 0 to prevent draining the sequencer.
    #[error("tx value not zero")]
    ValueNotZero,
    /// The from field is set on the transaction.
    ///
    /// Requests with the from field are rejected, since it is implied that it will always be the
    /// sequencer.
    #[error("tx from field is set")]
    FromSet,
    /// The nonce field is set on the transaction.
    ///
    /// Requests with the nonce field set are rejected, as this is managed by the sequencer.
    #[error("tx nonce is set")]
    NonceSet,
    /// An authorization item was invalid.
    ///
    /// The item is invalid if it tries to delegate an account to a contract that is not
    /// whitelisted.
    #[error("invalid authorization address")]
    InvalidAuthorization,
    /// The to field of the transaction was invalid.
    ///
    /// The destination is invalid if:
    ///
    /// - There is no bytecode at the destination, or
    /// - The bytecode is not an EIP-7702 delegation designator, or
    /// - The delegation designator points to a contract that is not whitelisted
    #[error("the destination of the transaction is not a delegated account")]
    IllegalDestination,
    /// The transaction request was invalid.
    ///
    /// This is likely an internal error, as most of the request is built by the sequencer.
    #[error("invalid tx request")]
    InvalidTransactionRequest,
    /// An internal error occurred.
    #[error("internal error")]
    InternalError,
}
