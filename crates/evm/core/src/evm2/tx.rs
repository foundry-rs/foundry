//! Network-native transactions.
//!
//! Transactions stay in their network's Alloy form until execution: tools keep the selected
//! [`TransactionRequest`](alloy_network::Network::TransactionRequest) or signed
//! [`TxEnvelope`](alloy_network::Network::TxEnvelope), mutate common fields through Alloy's
//! [`TransactionBuilder`](alloy_network::TransactionBuilder), and convert once, right before
//! execution, into the family's evm2 transaction with
//! [`FoundryEvmNetwork`](super::FoundryEvmNetwork). Signed, resumed and replayed transactions
//! convert from exactly the payload that was signed; nothing is rebuilt from Ethereum-shaped
//! fields.

use alloy_consensus::{EthereumTxEnvelope, TxEip4844Variant};
use alloy_primitives::Signature;
use evm2::ethereum::{LazyTxEip7702, TxEnvelope};

/// What an execution-time transaction is priced and validated as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TxPayload {
    /// The transaction's own signed encoding, as broadcast or replayed.
    Signed,
    /// A simulation of an unsigned request.
    ///
    /// Networks that price fees over the encoded transaction, such as the OP-stack L1 data fee,
    /// charge nothing for it, and networks with simulation-only metadata, such as Tempo, mark the
    /// transaction as simulated.
    #[default]
    Simulated,
}

impl TxPayload {
    /// Returns whether this is a simulation of an unsigned request.
    pub const fn is_simulated(self) -> bool {
        matches!(self, Self::Simulated)
    }
}

/// Placeholder signature used to put an unsigned request into its network's envelope.
///
/// The sender is always supplied separately, so the signature is never recovered.
pub fn placeholder_signature() -> Signature {
    Signature::test_signature()
}

/// Converts a signed Ethereum envelope into evm2's Ethereum transaction.
///
/// Signed EIP-7702 authorizations are kept as signed, so their authorities are recovered during
/// execution exactly as on chain.
pub fn eth_tx(envelope: EthereumTxEnvelope<TxEip4844Variant>) -> TxEnvelope {
    match envelope {
        EthereumTxEnvelope::Legacy(tx) => TxEnvelope::Legacy(tx.strip_signature()),
        EthereumTxEnvelope::Eip2930(tx) => TxEnvelope::Eip2930(tx.strip_signature()),
        EthereumTxEnvelope::Eip1559(tx) => TxEnvelope::Eip1559(tx.strip_signature()),
        EthereumTxEnvelope::Eip4844(tx) => TxEnvelope::Eip4844(tx.strip_signature()),
        EthereumTxEnvelope::Eip7702(tx) => {
            TxEnvelope::Eip7702(LazyTxEip7702::from_recovered_authorizations(tx.strip_signature()))
        }
    }
}
