use alloy_network::Network;

#[cfg(feature = "optimism")]
mod optimism;
mod receipt;

use alloy_provider::fillers::{
    BlobGasFiller, ChainIdFiller, GasFiller, JoinFill, NonceFiller, RecommendedFillers,
};
#[cfg(feature = "optimism")]
pub use optimism::FoundryTransactionResponse;
pub use receipt::*;

/// Default JSON-RPC transaction response when the `optimism` feature is disabled.
#[cfg(not(feature = "optimism"))]
pub type FoundryTransactionResponse = alloy_rpc_types_eth::Transaction<crate::FoundryTxEnvelope>;

/// Foundry network type.
///
/// This network type supports Foundry-specific transaction types, including
/// op-stack deposit transactions, alongside standard Ethereum transaction types.
///
/// Note: This is a basic implementation ("for now") that provides the core Network
/// trait definitions. Full Foundry-specific RPC types will be implemented in future work.
/// Currently, this uses Ethereum's Network configuration as a compatibility layer.
#[derive(Debug, Clone, Copy)]
pub struct FoundryNetwork {
    _private: (),
}

// Use Ethereum's Network trait implementation as the basis.
// This provides compatibility with the alloy-network ecosystem while we build
// out Foundry-specific RPC types.
impl Network for FoundryNetwork {
    type TxType = crate::FoundryTxType;

    type TxEnvelope = crate::FoundryTxEnvelope;

    type UnsignedTx = crate::FoundryTypedTx;

    type ReceiptEnvelope = crate::FoundryReceiptEnvelope;

    type Header = alloy_consensus::Header;

    type TransactionRequest = crate::FoundryTransactionRequest;

    type TransactionResponse = FoundryTransactionResponse;

    type ReceiptResponse = crate::FoundryTxReceipt;

    type HeaderResponse = alloy_rpc_types_eth::Header;

    type BlockResponse =
        alloy_rpc_types_eth::Block<Self::TransactionResponse, Self::HeaderResponse>;
}

impl RecommendedFillers for FoundryNetwork {
    type RecommendedFillers =
        JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>;

    fn recommended_fillers() -> Self::RecommendedFillers {
        Default::default()
    }
}
