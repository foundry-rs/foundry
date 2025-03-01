mod builder;

mod either;
pub use either::{AnyTxEnvelope, AnyTypedTransaction};

mod unknowns;
pub use unknowns::{AnyTxType, UnknownTxEnvelope, UnknownTypedTransaction};

pub use alloy_consensus_any::{AnyHeader, AnyReceiptEnvelope};

use crate::Network;
pub use alloy_rpc_types_any::{AnyRpcHeader, AnyTransactionReceipt};
use alloy_rpc_types_eth::{Block, Transaction, TransactionRequest};
use alloy_serde::WithOtherFields;

/// A catch-all block type for handling blocks on multiple networks.
pub type AnyRpcBlock =
    WithOtherFields<Block<WithOtherFields<Transaction<AnyTxEnvelope>>, AnyRpcHeader>>;

/// A catch-all transaction type for handling transactions on multiple networks.
pub type AnyRpcTransaction = WithOtherFields<Transaction<AnyTxEnvelope>>;

/// Types for a catch-all network.
///
/// `AnyNetwork`'s associated types allow for many different types of
/// transactions, using catch-all fields. This [`Network`] should be used
/// only when the application needs to support multiple networks via the same
/// codepaths without knowing the networks at compile time.
///
/// ## Rough Edges
///
/// Supporting arbitrary unknown types is hard, and users of this network
/// should be aware of the following:
///
/// - The implementation of [`Decodable2718`] for [`AnyTxEnvelope`] will not work for non-Ethereum
///   transaction types. It will succesfully decode an Ethereum [`TxEnvelope`], but will decode only
///   the type for any unknown transaction type. It will also leave the buffer unconsumed, which
///   will cause further deserialization to produce erroneous results.
/// - The implementation of [`Encodable2718`] for [`AnyTypedTransaction`] will not work for
///   non-Ethereum transaction types. It will encode the type for any unknown transaction type, but
///   will not encode any other fields. This is symmetric with the decoding behavior, but still
///   erroneous.
/// - The [`TransactionRequest`] will build ONLY Ethereum types. It will error when attempting to
///   build any unknown type.
/// - The [`Network::TransactionResponse`] may deserialize unknown metadata fields into the inner
///   [`AnyTxEnvelope`], rather than into the outer [`WithOtherFields`].
///
/// [`Decodable2718`]: alloy_eips::eip2718::Decodable2718
/// [`Encodable2718`]: alloy_eips::eip2718::Encodable2718
/// [`TxEnvelope`]: alloy_consensus::TxEnvelope
#[derive(Clone, Copy, Debug)]
pub struct AnyNetwork {
    _private: (),
}

impl Network for AnyNetwork {
    type TxType = AnyTxType;

    type TxEnvelope = AnyTxEnvelope;

    type UnsignedTx = AnyTypedTransaction;

    type ReceiptEnvelope = AnyReceiptEnvelope;

    type Header = AnyHeader;

    type TransactionRequest = WithOtherFields<TransactionRequest>;

    type TransactionResponse = AnyRpcTransaction;

    type ReceiptResponse = AnyTransactionReceipt;

    type HeaderResponse = AnyRpcHeader;

    type BlockResponse = AnyRpcBlock;
}
