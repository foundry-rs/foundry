//! RPC types for transactions

use alloy_consensus::{
    Signed, TxEip1559, TxEip2930, TxEip4844, TxEip4844Variant, TxEip7702, TxEnvelope, TxLegacy,
    Typed2718,
};
use alloy_eips::{eip2718::Encodable2718, eip7702::SignedAuthorization};
use alloy_network_primitives::TransactionResponse;
use alloy_primitives::{Address, BlockHash, Bytes, ChainId, TxKind, B256, U256};

pub use alloy_consensus::BlobTransactionSidecar;
pub use alloy_eips::{
    eip2930::{AccessList, AccessListItem, AccessListResult},
    eip7702::Authorization,
};

pub use alloy_consensus::transaction::TransactionInfo;

mod error;
pub use error::ConversionError;

mod receipt;
pub use receipt::TransactionReceipt;

pub mod request;
pub use request::{TransactionInput, TransactionRequest};

pub use alloy_consensus::{
    Receipt, ReceiptEnvelope, ReceiptWithBloom, Transaction as TransactionTrait,
};
pub use alloy_consensus_any::AnyReceiptEnvelope;

/// Transaction object used in RPC
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(all(any(test, feature = "arbitrary"), feature = "k256"), derive(arbitrary::Arbitrary))]
#[cfg_attr(
    feature = "serde",
    serde(
        into = "tx_serde::TransactionSerdeHelper<T>",
        try_from = "tx_serde::TransactionSerdeHelper<T>",
        bound = "T: TransactionTrait + Clone + serde::Serialize + serde::de::DeserializeOwned"
    )
)]
#[doc(alias = "Tx")]
pub struct Transaction<T = TxEnvelope> {
    /// The inner transaction object
    pub inner: T,

    /// Hash of block where transaction was included, `None` if pending
    pub block_hash: Option<BlockHash>,

    /// Number of block where transaction was included, `None` if pending
    pub block_number: Option<u64>,

    /// Transaction Index
    pub transaction_index: Option<u64>,

    /// Deprecated effective gas price value.
    pub effective_gas_price: Option<u128>,

    /// Sender
    pub from: Address,
}

impl<T> Transaction<T> {
    /// Applies the given closure to the inner transaction type.
    pub fn map<Tx>(self, f: impl FnOnce(T) -> Tx) -> Transaction<Tx> {
        let Self { inner, block_hash, block_number, transaction_index, effective_gas_price, from } =
            self;
        Transaction {
            inner: f(inner),
            block_hash,
            block_number,
            transaction_index,
            effective_gas_price,
            from,
        }
    }

    /// Applies the given fallible closure to the inner transactions.
    pub fn try_map<Tx, E>(self, f: impl FnOnce(T) -> Result<Tx, E>) -> Result<Transaction<Tx>, E> {
        let Self { inner, block_hash, block_number, transaction_index, effective_gas_price, from } =
            self;
        Ok(Transaction {
            inner: f(inner)?,
            block_hash,
            block_number,
            transaction_index,
            effective_gas_price,
            from,
        })
    }
}

impl<T> AsRef<T> for Transaction<T> {
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

impl<T> Transaction<T>
where
    T: TransactionTrait,
{
    /// Returns true if the transaction is a legacy or 2930 transaction.
    pub fn is_legacy_gas(&self) -> bool {
        self.inner.gas_price().is_some()
    }
}

impl<T> Transaction<T>
where
    T: TransactionTrait + Encodable2718,
{
    /// Returns the [`TransactionInfo`] for this transaction.
    ///
    /// This contains various metadata about the transaction and block context if available.
    pub fn info(&self) -> TransactionInfo {
        TransactionInfo {
            hash: Some(self.tx_hash()),
            index: self.transaction_index,
            block_hash: self.block_hash,
            block_number: self.block_number,
            // We don't know the base fee of the block when we're constructing this from
            // `Transaction`
            base_fee: None,
        }
    }
}

impl<T> Transaction<T>
where
    T: Into<TransactionRequest>,
{
    /// Converts [Transaction] into [TransactionRequest].
    ///
    /// During this conversion data for [TransactionRequest::sidecar] is not
    /// populated as it is not part of [Transaction].
    pub fn into_request(self) -> TransactionRequest {
        self.inner.into()
    }
}

impl<T> From<&Transaction<T>> for TransactionInfo
where
    T: TransactionTrait + Encodable2718,
{
    fn from(tx: &Transaction<T>) -> Self {
        tx.info()
    }
}

impl TryFrom<Transaction> for Signed<TxLegacy> {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx.inner {
            TxEnvelope::Legacy(tx) => Ok(tx),
            _ => {
                Err(ConversionError::Custom(format!("expected Legacy, got {}", tx.inner.tx_type())))
            }
        }
    }
}

impl TryFrom<Transaction> for Signed<TxEip1559> {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx.inner {
            TxEnvelope::Eip1559(tx) => Ok(tx),
            _ => Err(ConversionError::Custom(format!(
                "expected Eip1559, got {}",
                tx.inner.tx_type()
            ))),
        }
    }
}

impl TryFrom<Transaction> for Signed<TxEip2930> {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx.inner {
            TxEnvelope::Eip2930(tx) => Ok(tx),
            _ => Err(ConversionError::Custom(format!(
                "expected Eip2930, got {}",
                tx.inner.tx_type()
            ))),
        }
    }
}

impl TryFrom<Transaction> for Signed<TxEip4844> {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        let tx: Signed<TxEip4844Variant> = tx.try_into()?;

        let (tx, sig, hash) = tx.into_parts();

        Ok(Self::new_unchecked(tx.into(), sig, hash))
    }
}

impl TryFrom<Transaction> for Signed<TxEip4844Variant> {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx.inner {
            TxEnvelope::Eip4844(tx) => Ok(tx),
            _ => Err(ConversionError::Custom(format!(
                "expected TxEip4844Variant, got {}",
                tx.inner.tx_type()
            ))),
        }
    }
}

impl TryFrom<Transaction> for Signed<TxEip7702> {
    type Error = ConversionError;

    fn try_from(tx: Transaction) -> Result<Self, Self::Error> {
        match tx.inner {
            TxEnvelope::Eip7702(tx) => Ok(tx),
            _ => Err(ConversionError::Custom(format!(
                "expected Eip7702, got {}",
                tx.inner.tx_type()
            ))),
        }
    }
}

impl From<Transaction> for TxEnvelope {
    fn from(tx: Transaction) -> Self {
        tx.inner
    }
}

impl<T: TransactionTrait> TransactionTrait for Transaction<T> {
    fn chain_id(&self) -> Option<ChainId> {
        self.inner.chain_id()
    }

    fn nonce(&self) -> u64 {
        self.inner.nonce()
    }

    fn gas_limit(&self) -> u64 {
        self.inner.gas_limit()
    }

    fn gas_price(&self) -> Option<u128> {
        self.inner.gas_price()
    }

    fn max_fee_per_gas(&self) -> u128 {
        self.inner.max_fee_per_gas()
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.inner.max_priority_fee_per_gas()
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.inner.max_fee_per_blob_gas()
    }

    fn priority_fee_or_price(&self) -> u128 {
        self.inner.priority_fee_or_price()
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        self.inner.effective_gas_price(base_fee)
    }

    fn is_dynamic_fee(&self) -> bool {
        self.inner.is_dynamic_fee()
    }

    fn kind(&self) -> TxKind {
        self.inner.kind()
    }

    fn is_create(&self) -> bool {
        self.inner.is_create()
    }

    fn value(&self) -> U256 {
        self.inner.value()
    }

    fn input(&self) -> &Bytes {
        self.inner.input()
    }

    fn access_list(&self) -> Option<&AccessList> {
        self.inner.access_list()
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.inner.blob_versioned_hashes()
    }

    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        self.inner.authorization_list()
    }
}

impl<T: TransactionTrait + Encodable2718> TransactionResponse for Transaction<T> {
    fn tx_hash(&self) -> B256 {
        self.inner.trie_hash()
    }

    fn block_hash(&self) -> Option<BlockHash> {
        self.block_hash
    }

    fn block_number(&self) -> Option<u64> {
        self.block_number
    }

    fn transaction_index(&self) -> Option<u64> {
        self.transaction_index
    }

    fn from(&self) -> Address {
        self.from
    }
}

impl<T: Typed2718> Typed2718 for Transaction<T> {
    fn ty(&self) -> u8 {
        self.inner.ty()
    }
}

#[cfg(feature = "serde")]
mod tx_serde {
    //! Helper module for serializing and deserializing OP [`Transaction`].
    //!
    //! This is needed because we might need to deserialize the `gasPrice` field into both
    //! [`crate::Transaction::effective_gas_price`] and [`alloy_consensus::TxLegacy::gas_price`].
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Helper struct which will be flattened into the transaction and will only contain `gasPrice`
    /// field if inner [`TxEnvelope`] did not consume it.
    #[derive(Serialize, Deserialize)]
    struct MaybeGasPrice {
        #[serde(
            default,
            rename = "gasPrice",
            skip_serializing_if = "Option::is_none",
            with = "alloy_serde::quantity::opt"
        )]
        pub effective_gas_price: Option<u128>,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(crate) struct TransactionSerdeHelper<T> {
        #[serde(flatten)]
        inner: T,
        #[serde(default)]
        block_hash: Option<BlockHash>,
        #[serde(default, with = "alloy_serde::quantity::opt")]
        block_number: Option<u64>,
        #[serde(default, with = "alloy_serde::quantity::opt")]
        transaction_index: Option<u64>,
        /// Sender
        from: Address,

        #[serde(flatten)]
        gas_price: MaybeGasPrice,
    }

    impl<T: TransactionTrait> From<Transaction<T>> for TransactionSerdeHelper<T> {
        fn from(value: Transaction<T>) -> Self {
            let Transaction {
                inner,
                block_hash,
                block_number,
                transaction_index,
                effective_gas_price,
                from,
            } = value;

            // if inner transaction has its own `gasPrice` don't serialize it in this struct.
            let effective_gas_price = effective_gas_price.filter(|_| inner.gas_price().is_none());

            Self {
                inner,
                block_hash,
                block_number,
                transaction_index,
                from,
                gas_price: MaybeGasPrice { effective_gas_price },
            }
        }
    }

    impl<T: TransactionTrait> TryFrom<TransactionSerdeHelper<T>> for Transaction<T> {
        type Error = serde_json::Error;

        fn try_from(value: TransactionSerdeHelper<T>) -> Result<Self, Self::Error> {
            let TransactionSerdeHelper {
                inner,
                block_hash,
                block_number,
                transaction_index,
                from,
                gas_price,
            } = value;

            // Try to get `gasPrice` field from inner envelope or from `MaybeGasPrice`, otherwise
            // return error
            let effective_gas_price = inner.gas_price().or(gas_price.effective_gas_price);

            Ok(Self {
                inner,
                block_hash,
                block_number,
                transaction_index,
                from,
                effective_gas_price,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "serde")]
    fn into_request_legacy() {
        // cast rpc eth_getTransactionByHash
        // 0xe9e91f1ee4b56c0df2e9f06c2b8c27c6076195a88a7b8537ba8313d80e6f124e --rpc-url mainnet
        let rpc_tx = r#"{"blockHash":"0x8e38b4dbf6b11fcc3b9dee84fb7986e29ca0a02cecd8977c161ff7333329681e","blockNumber":"0xf4240","hash":"0xe9e91f1ee4b56c0df2e9f06c2b8c27c6076195a88a7b8537ba8313d80e6f124e","transactionIndex":"0x1","type":"0x0","nonce":"0x43eb","input":"0x","r":"0x3b08715b4403c792b8c7567edea634088bedcd7f60d9352b1f16c69830f3afd5","s":"0x10b9afb67d2ec8b956f0e1dbc07eb79152904f3a7bf789fc869db56320adfe09","chainId":"0x0","v":"0x1c","gas":"0xc350","from":"0x32be343b94f860124dc4fee278fdcbd38c102d88","to":"0xdf190dc7190dfba737d7777a163445b7fff16133","value":"0x6113a84987be800","gasPrice":"0xdf8475800"}"#;

        let tx = serde_json::from_str::<Transaction>(rpc_tx).unwrap();
        let request = tx.into_request();
        assert!(request.gas_price.is_some());
        assert!(request.max_fee_per_gas.is_none());
    }

    #[test]
    #[cfg(feature = "serde")]
    fn into_request_eip1559() {
        // cast rpc eth_getTransactionByHash
        // 0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c --rpc-url mainnet
        let rpc_tx = r#"{"blockHash":"0x883f974b17ca7b28cb970798d1c80f4d4bb427473dc6d39b2a7fe24edc02902d","blockNumber":"0xe26e6d","hash":"0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c","accessList":[],"transactionIndex":"0xad","type":"0x2","nonce":"0x16d","input":"0x5ae401dc00000000000000000000000000000000000000000000000000000000628ced5b000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000016000000000000000000000000000000000000000000000000000000000000000e442712a6700000000000000000000000000000000000000000000b3ff1489674e11c40000000000000000000000000000000000000000000000000000004a6ed55bbcc18000000000000000000000000000000000000000000000000000000000000000800000000000000000000000003cf412d970474804623bb4e3a42de13f9bca54360000000000000000000000000000000000000000000000000000000000000002000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000003a75941763f31c930b19c041b709742b0b31ebb600000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000412210e8a00000000000000000000000000000000000000000000000000000000","r":"0x7f2153019a74025d83a73effdd91503ceecefac7e35dd933adc1901c875539aa","s":"0x334ab2f714796d13c825fddf12aad01438db3a8152b2fe3ef7827707c25ecab3","chainId":"0x1","v":"0x0","gas":"0x46a02","maxPriorityFeePerGas":"0x59682f00","from":"0x3cf412d970474804623bb4e3a42de13f9bca5436","to":"0x68b3465833fb72a70ecdf485e0e4c7bd8665fc45","maxFeePerGas":"0x7fc1a20a8","value":"0x4a6ed55bbcc180","gasPrice":"0x50101df3a"}"#;

        let tx = serde_json::from_str::<Transaction>(rpc_tx).unwrap();
        let request = tx.into_request();
        assert!(request.gas_price.is_none());
        assert!(request.max_fee_per_gas.is_some());
    }

    #[test]
    fn serde_tx_from_contract_mod() {
        let rpc_tx = r#"{"hash":"0x018b2331d461a4aeedf6a1f9cc37463377578244e6a35216057a8370714e798f","nonce":"0x1","blockHash":"0x6e4e53d1de650d5a5ebed19b38321db369ef1dc357904284ecf4d89b8834969c","blockNumber":"0x2","transactionIndex":"0x0","from":"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266","to":"0x5fbdb2315678afecb367f032d93f642f64180aa3","value":"0x0","gasPrice":"0x3a29f0f8","gas":"0x1c9c380","maxFeePerGas":"0xba43b7400","maxPriorityFeePerGas":"0x5f5e100","input":"0xd09de08a","r":"0xd309309a59a49021281cb6bb41d164c96eab4e50f0c1bd24c03ca336e7bc2bb7","s":"0x28a7f089143d0a1355ebeb2a1b9f0e5ad9eca4303021c1400d61bc23c9ac5319","v":"0x0","yParity":"0x0","chainId":"0x7a69","accessList":[],"type":"0x2"}"#;

        let tx = serde_json::from_str::<Transaction>(rpc_tx).unwrap();
        assert_eq!(tx.block_number, Some(2));
    }

    #[test]
    fn test_gas_price_present() {
        let blob_rpc_tx = r#"{"blockHash":"0x1732a5fe86d54098c431fa4fea34387b650e41dbff65ca554370028172fcdb6a","blockNumber":"0x3","from":"0x7435ed30a8b4aeb0877cef0c6e8cffe834eb865f","gas":"0x186a0","gasPrice":"0x281d620e","maxFeePerGas":"0x281d620e","maxPriorityFeePerGas":"0x1","maxFeePerBlobGas":"0x20000","hash":"0xb0ebf0d8fca6724d5111d0be9ac61f0e7bf174208e0fafcb653f337c72465b83","input":"0xdc4c8669df128318656d6974","nonce":"0x8","to":"0x7dcd17433742f4c0ca53122ab541d0ba67fc27df","transactionIndex":"0x0","value":"0x3","type":"0x3","accessList":[{"address":"0x7dcd17433742f4c0ca53122ab541d0ba67fc27df","storageKeys":["0x0000000000000000000000000000000000000000000000000000000000000000","0x462708a3c1cd03b21605715d090136df64e227f7e7792f74bb1bd7a8288f8801"]}],"chainId":"0xc72dd9d5e883e","blobVersionedHashes":["0x015a4cab4911426699ed34483de6640cf55a568afc5c5edffdcbd8bcd4452f68"],"v":"0x0","r":"0x478385a47075dd6ba56300b623038052a6e4bb03f8cfc53f367712f1c1d3e7de","s":"0x2f79ed9b154b0af2c97ddfc1f4f76e6c17725713b6d44ea922ca4c6bbc20775c","yParity":"0x0"}"#;
        let legacy_rpc_tx = r#"{"blockHash":"0x7e5d03caac4eb2b613ae9c919ef3afcc8ed0e384f31ee746381d3c8739475d2a","blockNumber":"0x4","from":"0x7435ed30a8b4aeb0877cef0c6e8cffe834eb865f","gas":"0x5208","gasPrice":"0x23237dee","hash":"0x3f38cdc805c02e152bfed34471a3a13a786fed436b3aec0c3eca35d23e2cdd2c","input":"0x","nonce":"0xc","to":"0x4dde844b71bcdf95512fb4dc94e84fb67b512ed8","transactionIndex":"0x0","value":"0x1","type":"0x0","chainId":"0xc72dd9d5e883e","v":"0x18e5bb3abd10a0","r":"0x3d61f5d7e93eecd0669a31eb640ab3349e9e5868a44c2be1337c90a893b51990","s":"0xc55f44ba123af37d0e73ed75e578647c3f473805349936f64ea902ea9e03bc7"}"#;

        let blob_tx = serde_json::from_str::<Transaction>(blob_rpc_tx).unwrap();
        assert_eq!(blob_tx.block_number, Some(3));
        assert_eq!(blob_tx.effective_gas_price, Some(0x281d620e));

        let legacy_tx = serde_json::from_str::<Transaction>(legacy_rpc_tx).unwrap();
        assert_eq!(legacy_tx.block_number, Some(4));
        assert_eq!(legacy_tx.effective_gas_price, Some(0x23237dee));
    }

    // <https://github.com/alloy-rs/alloy/issues/1643>
    #[test]
    fn deserialize_7702_v() {
        let raw = r#"{"blockHash":"0xb14eac260f0cb7c3bbf4c9ff56034defa4f566780ed3e44b7a79b6365d02887c","blockNumber":"0xb022","from":"0x6d2d4e1c2326a069f36f5d6337470dc26adb7156","gas":"0xf8ac","gasPrice":"0xe07899f","maxFeePerGas":"0xe0789a0","maxPriorityFeePerGas":"0xe078998","hash":"0xadc3f24d05f05f1065debccb1c4b033eaa35917b69b343d88d9062cdf8ecad83","input":"0x","nonce":"0x1a","to":"0x6d2d4e1c2326a069f36f5d6337470dc26adb7156","transactionIndex":"0x0","value":"0x0","type":"0x4","accessList":[],"chainId":"0x1a5ee289c","authorizationList":[{"chainId":"0x1a5ee289c","address":"0x529f773125642b12a44bd543005650989eceaa2a","nonce":"0x1a","v":"0x0","r":"0x9b3de20cf8bd07f3c5c55c38c920c146f081bc5ab4580d0c87786b256cdab3c2","s":"0x74841956f4832bace3c02aed34b8f0a2812450da3728752edbb5b5e1da04497"}],"v":"0x1","r":"0xb3bf7d6877864913bba04d6f93d98009a5af16ee9c12295cd634962a2346b67c","s":"0x31ca4a874afa964ec7643e58c6b56b35b1bcc7698eb1b5e15e61e78b353bd42d","yParity":"0x1"}"#;
        let tx = serde_json::from_str::<Transaction>(raw).unwrap();
        assert!(tx.inner.is_eip7702());
    }
}
