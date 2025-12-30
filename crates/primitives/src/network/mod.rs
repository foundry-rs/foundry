use alloy_network::Network;

mod receipt;

pub use receipt::*;

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

    type TransactionResponse = op_alloy_rpc_types::Transaction<crate::FoundryTxEnvelope>;

    type ReceiptResponse = crate::FoundryTxReceipt;

    type HeaderResponse = alloy_rpc_types_eth::Header;

    type BlockResponse =
        alloy_rpc_types_eth::Block<Self::TransactionResponse, Self::HeaderResponse>;
}
