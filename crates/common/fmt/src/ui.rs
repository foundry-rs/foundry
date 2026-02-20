//! Helper trait and functions to format Ethereum types.

use alloy_consensus::{
    BlockHeader, Eip658Value, Receipt, ReceiptWithBloom, Transaction as TxTrait, TxEnvelope,
    TxType, Typed2718,
};
use alloy_network::{
    AnyHeader, AnyReceiptEnvelope, AnyRpcBlock, AnyRpcHeader, AnyRpcTransaction,
    AnyTransactionReceipt, AnyTxEnvelope, BlockResponse, Network, ReceiptResponse,
    primitives::HeaderResponse,
};
use alloy_primitives::{Address, Bloom, Bytes, FixedBytes, I256, U8, U64, U256, Uint, hex};
use alloy_rpc_types::{
    AccessListItem, Block, BlockTransactions, Header, Log, Transaction, TransactionReceipt,
};
use alloy_serde::{OtherFields, WithOtherFields};
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope, FoundryTxReceipt};
use revm::context_interface::transaction::SignedAuthorization;
use serde::Deserialize;

/// length of the name column for pretty formatting `{:>20}{value}`
const NAME_COLUMN_LEN: usize = 20usize;

/// Helper trait to format Ethereum types.
///
/// # Examples
///
/// ```
/// use foundry_common_fmt::UIfmt;
///
/// let boolean: bool = true;
/// let string = boolean.pretty();
/// ```
pub trait UIfmt {
    /// Return a prettified string version of the value
    fn pretty(&self) -> String;
}

impl<T: UIfmt> UIfmt for &T {
    fn pretty(&self) -> String {
        (*self).pretty()
    }
}

impl<T: UIfmt> UIfmt for Option<T> {
    fn pretty(&self) -> String {
        if let Some(inner) = self { inner.pretty() } else { String::new() }
    }
}

impl<T: UIfmt> UIfmt for [T] {
    fn pretty(&self) -> String {
        if !self.is_empty() {
            let mut s = String::with_capacity(self.len() * 64);
            s.push_str("[\n");
            for item in self {
                for line in item.pretty().lines() {
                    s.push('\t');
                    s.push_str(line);
                    s.push('\n');
                }
            }
            s.push(']');
            s
        } else {
            "[]".to_string()
        }
    }
}

impl UIfmt for String {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for u64 {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for u128 {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for bool {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl<const BITS: usize, const LIMBS: usize> UIfmt for Uint<BITS, LIMBS> {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for I256 {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for Address {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for Bloom {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for TxType {
    fn pretty(&self) -> String {
        (*self as u8).to_string()
    }
}

impl UIfmt for Vec<u8> {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl UIfmt for Bytes {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl<const N: usize> UIfmt for [u8; N] {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl<const N: usize> UIfmt for FixedBytes<N> {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl UIfmt for [u8] {
    fn pretty(&self) -> String {
        hex::encode_prefixed(self)
    }
}

impl UIfmt for Eip658Value {
    fn pretty(&self) -> String {
        match self {
            Self::Eip658(status) => if *status { "1 (success)" } else { "0 (failed)" }.to_string(),
            Self::PostState(state) => state.pretty(),
        }
    }
}

impl UIfmt for AnyTransactionReceipt {
    fn pretty(&self) -> String {
        let Self {
            inner:
                TransactionReceipt {
                    transaction_hash,
                    transaction_index,
                    block_hash,
                    block_number,
                    from,
                    to,
                    gas_used,
                    contract_address,
                    effective_gas_price,
                    inner:
                        AnyReceiptEnvelope {
                            r#type: transaction_type,
                            inner:
                                ReceiptWithBloom {
                                    receipt: Receipt { status, cumulative_gas_used, logs },
                                    logs_bloom,
                                },
                        },
                    blob_gas_price,
                    blob_gas_used,
                },
            other,
        } = self;

        let mut pretty = format!(
            "
blockHash            {}
blockNumber          {}
contractAddress      {}
cumulativeGasUsed    {}
effectiveGasPrice    {}
from                 {}
gasUsed              {}
logs                 {}
logsBloom            {}
root                 {}
status               {}
transactionHash      {}
transactionIndex     {}
type                 {}
blobGasPrice         {}
blobGasUsed          {}",
            block_hash.pretty(),
            block_number.pretty(),
            contract_address.pretty(),
            cumulative_gas_used.pretty(),
            effective_gas_price.pretty(),
            from.pretty(),
            gas_used.pretty(),
            serde_json::to_string(&logs).unwrap(),
            logs_bloom.pretty(),
            self.state_root().pretty(),
            status.pretty(),
            transaction_hash.pretty(),
            transaction_index.pretty(),
            transaction_type,
            blob_gas_price.pretty(),
            blob_gas_used.pretty()
        );

        if let Some(to) = to {
            pretty.push_str(&format!("\nto                   {}", to.pretty()));
        }

        // additional captured fields
        pretty.push_str(&other.pretty());

        pretty
    }
}

impl UIfmt for Log {
    fn pretty(&self) -> String {
        format!(
            "
address: {}
blockHash: {}
blockNumber: {}
data: {}
logIndex: {}
removed: {}
topics: {}
transactionHash: {}
transactionIndex: {}",
            self.address().pretty(),
            self.block_hash.pretty(),
            self.block_number.pretty(),
            self.data().data.pretty(),
            self.log_index.pretty(),
            self.removed.pretty(),
            self.topics().pretty(),
            self.transaction_hash.pretty(),
            self.transaction_index.pretty(),
        )
    }
}

impl<T: UIfmt> UIfmt for Block<T, Header<AnyHeader>> {
    fn pretty(&self) -> String {
        format!(
            "
{}
transactions:        {}",
            pretty_generic_header_response(&self.header),
            self.transactions.pretty()
        )
    }
}

impl<T: UIfmt> UIfmt for BlockTransactions<T> {
    fn pretty(&self) -> String {
        match self {
            Self::Hashes(hashes) => hashes.pretty(),
            Self::Full(transactions) => transactions.pretty(),
            Self::Uncle => String::new(),
        }
    }
}

impl UIfmt for OtherFields {
    fn pretty(&self) -> String {
        let mut s = String::with_capacity(self.len() * 30);
        if !self.is_empty() {
            s.push('\n');
        }
        for (key, value) in self {
            let val = EthValue::from(value.clone()).pretty();
            let offset = NAME_COLUMN_LEN.saturating_sub(key.len());
            s.push_str(key);
            s.extend(std::iter::repeat_n(' ', offset + 1));
            s.push_str(&val);
            s.push('\n');
        }
        s
    }
}

impl UIfmt for AccessListItem {
    fn pretty(&self) -> String {
        let mut s = String::with_capacity(42 + self.storage_keys.len() * 66);
        s.push_str(self.address.pretty().as_str());
        s.push_str(" => ");
        s.push_str(self.storage_keys.pretty().as_str());
        s
    }
}

impl UIfmt for TxEnvelope {
    fn pretty(&self) -> String {
        match &self {
            Self::Eip2930(tx) => format!(
                "
accessList           {}
chainId              {}
gasLimit             {}
gasPrice             {}
hash                 {}
input                {}
nonce                {}
r                    {}
s                    {}
to                   {}
type                 {}
value                {}
yParity              {}",
                self.access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.chain_id().pretty(),
                self.gas_limit().pretty(),
                self.gas_price().pretty(),
                self.tx_hash().pretty(),
                self.input().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            Self::Eip1559(tx) => format!(
                "
accessList           {}
chainId              {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
type                 {}
value                {}
yParity              {}",
                self.access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.chain_id().pretty(),
                self.gas_limit().pretty(),
                self.tx_hash().pretty(),
                self.input().pretty(),
                self.max_fee_per_gas().pretty(),
                self.max_priority_fee_per_gas().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            Self::Eip4844(tx) => format!(
                "
accessList           {}
blobVersionedHashes  {}
chainId              {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerBlobGas     {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
type                 {}
value                {}
yParity              {}",
                self.access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.blob_versioned_hashes().unwrap_or(&[]).pretty(),
                self.chain_id().pretty(),
                self.gas_limit().pretty(),
                self.tx_hash().pretty(),
                self.input().pretty(),
                self.max_fee_per_blob_gas().pretty(),
                self.max_fee_per_gas().pretty(),
                self.max_priority_fee_per_gas().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            Self::Eip7702(tx) => format!(
                "
accessList           {}
authorizationList    {}
chainId              {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
type                 {}
value                {}
yParity              {}",
                self.access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.authorization_list()
                    .as_ref()
                    .map(|l| l.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.chain_id().pretty(),
                self.gas_limit().pretty(),
                self.tx_hash().pretty(),
                self.input().pretty(),
                self.max_fee_per_gas().pretty(),
                self.max_priority_fee_per_gas().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            _ => format!(
                "
gas                  {}
gasPrice             {}
hash                 {}
input                {}
nonce                {}
r                    {}
s                    {}
to                   {}
type                 {}
v                    {}
value                {}",
                self.gas_limit().pretty(),
                self.gas_price().pretty(),
                self.tx_hash().pretty(),
                self.input().pretty(),
                self.nonce().pretty(),
                self.as_legacy()
                    .map(|tx| FixedBytes::from(tx.signature().r()).pretty())
                    .unwrap_or_default(),
                self.as_legacy()
                    .map(|tx| FixedBytes::from(tx.signature().s()).pretty())
                    .unwrap_or_default(),
                self.to().pretty(),
                self.ty(),
                self.as_legacy()
                    .map(|tx| (if tx.signature().v() { 1u64 } else { 0 }).pretty())
                    .unwrap_or_default(),
                self.value().pretty(),
            ),
        }
    }
}

impl UIfmt for AnyTxEnvelope {
    fn pretty(&self) -> String {
        match self {
            Self::Ethereum(envelop) => envelop.pretty(),
            Self::Unknown(tx) => {
                format!(
                    "
hash                 {}
type                 {}
{}
                    ",
                    tx.hash.pretty(),
                    tx.ty(),
                    tx.inner.fields.pretty().trim_start(),
                )
            }
        }
    }
}
impl UIfmt for Transaction {
    fn pretty(&self) -> String {
        match &self.inner.inner() {
            TxEnvelope::Eip2930(tx) => format!(
                "
accessList           {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
gasPrice             {}
hash                 {}
input                {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}",
                self.inner
                    .access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id().pretty(),
                self.inner.signer().pretty(),
                self.gas_limit().pretty(),
                self.gas_price().pretty(),
                self.inner.tx_hash().pretty(),
                self.input().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.transaction_index.pretty(),
                self.inner.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            TxEnvelope::Eip1559(tx) => format!(
                "
accessList           {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}",
                self.inner
                    .access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id().pretty(),
                self.inner.signer().pretty(),
                self.gas_limit().pretty(),
                tx.hash().pretty(),
                self.input().pretty(),
                self.max_fee_per_gas().pretty(),
                self.max_priority_fee_per_gas().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.transaction_index.pretty(),
                self.inner.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            TxEnvelope::Eip4844(tx) => format!(
                "
accessList           {}
blobVersionedHashes  {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerBlobGas     {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}",
                self.inner
                    .access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.blob_versioned_hashes().unwrap_or(&[]).pretty(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id().pretty(),
                self.inner.signer().pretty(),
                self.gas_limit().pretty(),
                tx.hash().pretty(),
                self.input().pretty(),
                self.max_fee_per_blob_gas().pretty(),
                self.max_fee_per_gas().pretty(),
                self.max_priority_fee_per_gas().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.transaction_index.pretty(),
                self.inner.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            TxEnvelope::Eip7702(tx) => format!(
                "
accessList           {}
authorizationList    {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}",
                self.inner
                    .access_list()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.authorization_list()
                    .as_ref()
                    .map(|l| l.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id().pretty(),
                self.inner.signer().pretty(),
                self.gas_limit().pretty(),
                tx.hash().pretty(),
                self.input().pretty(),
                self.max_fee_per_gas().pretty(),
                self.max_priority_fee_per_gas().pretty(),
                self.nonce().pretty(),
                FixedBytes::from(tx.signature().r()).pretty(),
                FixedBytes::from(tx.signature().s()).pretty(),
                self.to().pretty(),
                self.transaction_index.pretty(),
                self.inner.ty(),
                self.value().pretty(),
                (if tx.signature().v() { 1u64 } else { 0 }).pretty(),
            ),
            _ => format!(
                "
blockHash            {}
blockNumber          {}
from                 {}
gas                  {}
gasPrice             {}
hash                 {}
input                {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
v                    {}
value                {}",
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.inner.signer().pretty(),
                self.gas_limit().pretty(),
                self.gas_price().pretty(),
                self.inner.tx_hash().pretty(),
                self.input().pretty(),
                self.nonce().pretty(),
                self.inner
                    .as_legacy()
                    .map(|tx| FixedBytes::from(tx.signature().r()).pretty())
                    .unwrap_or_default(),
                self.inner
                    .as_legacy()
                    .map(|tx| FixedBytes::from(tx.signature().s()).pretty())
                    .unwrap_or_default(),
                self.to().pretty(),
                self.transaction_index.pretty(),
                self.inner
                    .as_legacy()
                    .map(|tx| (if tx.signature().v() { 1u64 } else { 0 }).pretty())
                    .unwrap_or_default(),
                self.value().pretty(),
            ),
        }
    }
}

impl UIfmt for Transaction<AnyTxEnvelope> {
    fn pretty(&self) -> String {
        format!(
            "
blockHash            {}
blockNumber          {}
from                 {}
transactionIndex     {}
effectiveGasPrice    {}
{}
            ",
            self.block_hash.pretty(),
            self.block_number.pretty(),
            self.inner.signer().pretty(),
            self.transaction_index.pretty(),
            self.effective_gas_price.pretty(),
            self.inner.pretty().trim_start(),
        )
    }
}

impl UIfmt for AnyRpcBlock {
    fn pretty(&self) -> String {
        self.0.pretty()
    }
}

impl UIfmt for AnyRpcTransaction {
    fn pretty(&self) -> String {
        self.0.pretty()
    }
}

impl<T: UIfmt> UIfmt for WithOtherFields<T> {
    fn pretty(&self) -> String {
        format!("{}{}", self.inner.pretty(), self.other.pretty())
    }
}

/// Various numerical ethereum types used for pretty printing
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
#[expect(missing_docs)]
pub enum EthValue {
    U64(U64),
    Address(Address),
    U256(U256),
    U64Array(Vec<U64>),
    U256Array(Vec<U256>),
    Other(serde_json::Value),
}

impl From<serde_json::Value> for EthValue {
    fn from(val: serde_json::Value) -> Self {
        serde_json::from_value(val).expect("infallible")
    }
}

impl UIfmt for EthValue {
    fn pretty(&self) -> String {
        match self {
            Self::U64(num) => num.pretty(),
            Self::U256(num) => num.pretty(),
            Self::Address(addr) => addr.pretty(),
            Self::U64Array(arr) => arr.pretty(),
            Self::U256Array(arr) => arr.pretty(),
            Self::Other(val) => val.to_string().trim_matches('"').to_string(),
        }
    }
}

impl UIfmt for SignedAuthorization {
    fn pretty(&self) -> String {
        let signed_authorization = serde_json::to_string(self).unwrap_or("<invalid>".to_string());

        match self.recover_authority() {
            Ok(authority) => format!(
                "{{recoveredAuthority: {authority}, signedAuthority: {signed_authorization}}}",
            ),
            Err(e) => format!(
                "{{recoveredAuthority: <error: {e}>, signedAuthority: {signed_authorization}}}",
            ),
        }
    }
}

impl<T> UIfmt for FoundryReceiptEnvelope<T>
where
    T: UIfmt + Clone + core::fmt::Debug + PartialEq + Eq,
{
    fn pretty(&self) -> String {
        let receipt = self.as_receipt();
        let deposit_info = match self {
            Self::Deposit(d) => {
                format!(
                    "
depositNonce         {}
depositReceiptVersion {}",
                    d.receipt.deposit_nonce.pretty(),
                    d.receipt.deposit_receipt_version.pretty()
                )
            }
            _ => String::new(),
        };

        format!(
            "
status               {}
cumulativeGasUsed    {}
logs                 {}
logsBloom            {}
type                 {}{}",
            receipt.status.pretty(),
            receipt.cumulative_gas_used.pretty(),
            receipt.logs.pretty(),
            self.logs_bloom().pretty(),
            self.tx_type() as u8,
            deposit_info
        )
    }
}

impl UIfmt for FoundryTxReceipt {
    fn pretty(&self) -> String {
        let receipt = &self.0.inner;
        let other = &self.0.other;

        let mut pretty = format!(
            "
blockHash            {}
blockNumber          {}
contractAddress      {}
cumulativeGasUsed    {}
effectiveGasPrice    {}
from                 {}
gasUsed              {}
logs                 {}
logsBloom            {}
root                 {}
status               {}
transactionHash      {}
transactionIndex     {}
type                 {}
blobGasPrice         {}
blobGasUsed          {}",
            receipt.block_hash.pretty(),
            receipt.block_number.pretty(),
            receipt.contract_address.pretty(),
            receipt.inner.cumulative_gas_used().pretty(),
            receipt.effective_gas_price.pretty(),
            receipt.from.pretty(),
            receipt.gas_used.pretty(),
            serde_json::to_string(receipt.inner.logs()).unwrap(),
            receipt.inner.logs_bloom().pretty(),
            self.state_root().pretty(),
            receipt.inner.status().pretty(),
            receipt.transaction_hash.pretty(),
            receipt.transaction_index.pretty(),
            receipt.inner.tx_type() as u8,
            receipt.blob_gas_price.pretty(),
            receipt.blob_gas_used.pretty()
        );

        if let Some(to) = receipt.to {
            pretty.push_str(&format!("\nto                   {}", to.pretty()));
        }

        // additional captured fields
        pretty.push_str(&other.pretty());

        pretty
    }
}

pub trait UIfmtHeaderExt {
    fn size_pretty(&self) -> String;
}

impl UIfmtHeaderExt for Header {
    fn size_pretty(&self) -> String {
        self.size.pretty()
    }
}

impl UIfmtHeaderExt for AnyRpcHeader {
    fn size_pretty(&self) -> String {
        self.size.pretty()
    }
}

pub trait UIfmtSignatureExt {
    fn signature_pretty(&self) -> Option<(String, String, String)>;
}

impl UIfmtSignatureExt for TxEnvelope {
    fn signature_pretty(&self) -> Option<(String, String, String)> {
        let sig = self.signature();
        Some((
            FixedBytes::from(sig.r()).pretty(),
            FixedBytes::from(sig.s()).pretty(),
            U8::from_le_slice(&sig.as_bytes()[64..]).pretty(),
        ))
    }
}

impl UIfmtSignatureExt for AnyTxEnvelope {
    fn signature_pretty(&self) -> Option<(String, String, String)> {
        self.as_envelope().and_then(|envelope| envelope.signature_pretty())
    }
}

impl UIfmtSignatureExt for FoundryTxEnvelope {
    fn signature_pretty(&self) -> Option<(String, String, String)> {
        self.clone().try_into_eth().ok().and_then(|envelope| envelope.signature_pretty())
    }
}

pub trait UIfmtReceiptExt {
    fn logs_pretty(&self) -> String;
    fn logs_bloom_pretty(&self) -> String;
    fn tx_type_pretty(&self) -> String;
}

impl UIfmtReceiptExt for AnyTransactionReceipt {
    fn logs_pretty(&self) -> String {
        serde_json::to_string(&self.inner.inner.inner.receipt.logs).unwrap_or_default()
    }

    fn logs_bloom_pretty(&self) -> String {
        self.inner.inner.inner.logs_bloom.pretty()
    }

    fn tx_type_pretty(&self) -> String {
        self.inner.inner.r#type.to_string()
    }
}

impl UIfmtReceiptExt for FoundryTxReceipt {
    fn logs_pretty(&self) -> String {
        serde_json::to_string(self.0.inner.inner.logs()).unwrap_or_default()
    }

    fn logs_bloom_pretty(&self) -> String {
        self.0.inner.inner.logs_bloom().pretty()
    }

    fn tx_type_pretty(&self) -> String {
        (self.0.inner.inner.tx_type() as u8).to_string()
    }
}

/// Returns the `UiFmt::pretty()` formatted attribute of the transactions
pub fn get_pretty_tx_attr<N>(transaction: &N::TransactionResponse, attr: &str) -> Option<String>
where
    N: Network,
    N::TxEnvelope: UIfmtSignatureExt,
{
    let (r, s, v) = transaction.as_ref().signature_pretty().unwrap_or_default();
    match attr {
        "blockHash" | "block_hash" => {
            Some(alloy_network::TransactionResponse::block_hash(transaction).pretty())
        }
        "blockNumber" | "block_number" => {
            Some(alloy_network::TransactionResponse::block_number(transaction).pretty())
        }
        "from" => Some(alloy_network::TransactionResponse::from(transaction).pretty()),
        "gas" => Some(TxTrait::gas_limit(transaction).pretty()),
        "gasPrice" | "gas_price" => Some(TxTrait::max_fee_per_gas(transaction).pretty()),
        "hash" => Some(alloy_network::TransactionResponse::tx_hash(transaction).pretty()),
        "input" => Some(TxTrait::input(transaction).pretty()),
        "nonce" => Some(TxTrait::nonce(transaction).to_string()),
        "s" => Some(s),
        "r" => Some(r),
        "to" => Some(TxTrait::to(transaction).pretty()),
        "transactionIndex" | "transaction_index" => {
            Some(alloy_network::TransactionResponse::transaction_index(transaction).pretty())
        }
        "v" => Some(v),
        "value" => Some(TxTrait::value(transaction).pretty()),
        _ => None,
    }
}

pub fn get_pretty_block_attr<N>(block: &N::BlockResponse, attr: &str) -> Option<String>
where
    N: Network,
    N::BlockResponse: BlockResponse<Header = N::HeaderResponse>,
    N::HeaderResponse: UIfmtHeaderExt,
{
    match attr {
        "baseFeePerGas" | "base_fee_per_gas" => Some(block.header().base_fee_per_gas().pretty()),
        "difficulty" => Some(block.header().difficulty().pretty()),
        "extraData" | "extra_data" => Some(block.header().extra_data().pretty()),
        "gasLimit" | "gas_limit" => Some(block.header().gas_limit().pretty()),
        "gasUsed" | "gas_used" => Some(block.header().gas_used().pretty()),
        "hash" => Some(block.header().hash().pretty()),
        "logsBloom" | "logs_bloom" => Some(block.header().logs_bloom().pretty()),
        "miner" | "author" => Some(block.header().beneficiary().pretty()),
        "mixHash" | "mix_hash" => Some(block.header().mix_hash().pretty()),
        "nonce" => Some(block.header().nonce().pretty()),
        "number" => Some(block.header().number().pretty()),
        "parentHash" | "parent_hash" => Some(block.header().parent_hash().pretty()),
        "transactionsRoot" | "transactions_root" => {
            Some(block.header().transactions_root().pretty())
        }
        "receiptsRoot" | "receipts_root" => Some(block.header().receipts_root().pretty()),
        "sha3Uncles" | "sha_3_uncles" => Some(block.header().ommers_hash().pretty()),
        "size" => Some(block.header().size_pretty()),
        "stateRoot" | "state_root" => Some(block.header().state_root().pretty()),
        "timestamp" => Some(block.header().timestamp().pretty()),
        "totalDifficulty" | "total_difficult" => Some(block.header().difficulty().pretty()),
        "blobGasUsed" | "blob_gas_used" => Some(block.header().blob_gas_used().pretty()),
        "excessBlobGas" | "excess_blob_gas" => Some(block.header().excess_blob_gas().pretty()),
        "requestsHash" | "requests_hash" => Some(block.header().requests_hash().pretty()),
        other => {
            if let Some(value) = block.other_fields().and_then(|fields| fields.get(other)) {
                let val = EthValue::from(value.clone());
                return Some(val.pretty());
            }
            None
        }
    }
}

pub fn get_pretty_receipt_attr<N>(receipt: &N::ReceiptResponse, attr: &str) -> Option<String>
where
    N: Network,
    N::ReceiptResponse: ReceiptResponse + UIfmtReceiptExt,
{
    match attr {
        "blockHash" | "block_hash" => Some(receipt.block_hash().pretty()),
        "blockNumber" | "block_number" => Some(receipt.block_number().pretty()),
        "contractAddress" | "contract_address" => Some(receipt.contract_address().pretty()),
        "cumulativeGasUsed" | "cumulative_gas_used" => Some(receipt.cumulative_gas_used().pretty()),
        "effectiveGasPrice" | "effective_gas_price" => Some(receipt.effective_gas_price().pretty()),
        "from" => Some(receipt.from().pretty()),
        "gasUsed" | "gas_used" => Some(receipt.gas_used().pretty()),
        "logs" => Some(receipt.logs_pretty()),
        "logsBloom" | "logs_bloom" => Some(receipt.logs_bloom_pretty()),
        "root" | "stateRoot" | "state_root" => Some(receipt.state_root().pretty()),
        "status" | "statusCode" | "status_code" => Some(receipt.status().pretty()),
        "transactionHash" | "transaction_hash" => Some(receipt.transaction_hash().pretty()),
        "transactionIndex" | "transaction_index" => Some(receipt.transaction_index().pretty()),
        "to" => Some(receipt.to().pretty()),
        "type" | "transaction_type" => Some(receipt.tx_type_pretty()),
        "blobGasPrice" | "blob_gas_price" => Some(receipt.blob_gas_price().pretty()),
        "blobGasUsed" | "blob_gas_used" => Some(receipt.blob_gas_used().pretty()),
        _ => None,
    }
}

fn pretty_generic_header_response<H: HeaderResponse + UIfmtHeaderExt>(header: &H) -> String {
    format!(
        "
baseFeePerGas        {}
difficulty           {}
extraData            {}
gasLimit             {}
gasUsed              {}
hash                 {}
logsBloom            {}
miner                {}
mixHash              {}
nonce                {}
number               {}
parentHash           {}
parentBeaconRoot     {}
transactionsRoot     {}
receiptsRoot         {}
sha3Uncles           {}
size                 {}
stateRoot            {}
timestamp            {} ({})
withdrawalsRoot      {}
totalDifficulty      {}
blobGasUsed          {}
excessBlobGas        {}
requestsHash         {}",
        header.base_fee_per_gas().pretty(),
        header.difficulty().pretty(),
        header.extra_data().pretty(),
        header.gas_limit().pretty(),
        header.gas_used().pretty(),
        header.hash().pretty(),
        header.logs_bloom().pretty(),
        header.beneficiary().pretty(),
        header.mix_hash().pretty(),
        header.nonce().pretty(),
        header.number().pretty(),
        header.parent_hash().pretty(),
        header.parent_beacon_block_root().pretty(),
        header.transactions_root().pretty(),
        header.receipts_root().pretty(),
        header.ommers_hash().pretty(),
        header.size_pretty(),
        header.state_root().pretty(),
        header.timestamp().pretty(),
        fmt_timestamp(header.timestamp()),
        header.withdrawals_root().pretty(),
        header.difficulty().pretty(),
        header.blob_gas_used().pretty(),
        header.excess_blob_gas().pretty(),
        header.requests_hash().pretty(),
    )
}

/// Formats the timestamp to string
///
/// Assumes timestamp is seconds, but handles millis if it is too large
fn fmt_timestamp(timestamp: u64) -> String {
    // Tue Jan 19 2038 03:14:07 GMT+0000
    if timestamp > 2147483647 {
        // assume this is in millis, incorrectly set to millis by a node
        chrono::DateTime::from_timestamp_millis(timestamp as i64)
            .expect("block timestamp in range")
            .to_rfc3339()
    } else {
        // assume this is still in seconds
        chrono::DateTime::from_timestamp(timestamp as i64, 0)
            .expect("block timestamp in range")
            .to_rfc2822()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use alloy_rpc_types::Authorization;
    use foundry_primitives::FoundryNetwork;
    use similar_asserts::assert_eq;
    use std::str::FromStr;

    #[test]
    fn format_date_time() {
        // Fri Aug 29 2025 08:05:38 GMT+0000
        let timestamp = 1756454738u64;

        let datetime = fmt_timestamp(timestamp);
        assert_eq!(datetime, "Fri, 29 Aug 2025 08:05:38 +0000");
        let datetime = fmt_timestamp(timestamp * 1000);
        assert_eq!(datetime, "2025-08-29T08:05:38+00:00");
    }

    #[test]
    fn can_format_bytes32() {
        let val = hex::decode("7465737400000000000000000000000000000000000000000000000000000000")
            .unwrap();
        let mut b32 = [0u8; 32];
        b32.copy_from_slice(&val);

        assert_eq!(
            b32.pretty(),
            "0x7465737400000000000000000000000000000000000000000000000000000000"
        );
        let b: Bytes = val.into();
        assert_eq!(b.pretty(), b32.pretty());
    }

    #[test]
    fn can_pretty_print_optimism_tx() {
        let s = r#"
        {
        "blockHash": "0x02b853cf50bc1c335b70790f93d5a390a35a166bea9c895e685cc866e4961cae",
        "blockNumber": "0x1b4",
        "from": "0x3b179DcfC5fAa677044c27dCe958e4BC0ad696A6",
        "gas": "0x11cbbdc",
        "gasPrice": "0x0",
        "hash": "0x2642e960d3150244e298d52b5b0f024782253e6d0b2c9a01dd4858f7b4665a3f",
        "input": "0xd294f093",
        "nonce": "0xa2",
        "to": "0x4a16A42407AA491564643E1dfc1fd50af29794eF",
        "transactionIndex": "0x0",
        "value": "0x0",
        "v": "0x38",
        "r": "0x6fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2bee",
        "s": "0xe804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583",
        "queueOrigin": "sequencer",
        "txType": "",
        "l1TxOrigin": null,
        "l1BlockNumber": "0xc1a65c",
        "l1Timestamp": "0x60d34b60",
        "index": "0x1b3",
        "queueIndex": null,
        "rawTransaction": "0xf86681a28084011cbbdc944a16a42407aa491564643e1dfc1fd50af29794ef8084d294f09338a06fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2beea00e804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583"
    }
        "#;

        let tx: WithOtherFields<Transaction> = serde_json::from_str(s).unwrap();
        assert_eq!(tx.pretty().trim(),
                   r"
blockHash            0x02b853cf50bc1c335b70790f93d5a390a35a166bea9c895e685cc866e4961cae
blockNumber          436
from                 0x3b179DcfC5fAa677044c27dCe958e4BC0ad696A6
gas                  18660316
gasPrice             0
hash                 0x2642e960d3150244e298d52b5b0f024782253e6d0b2c9a01dd4858f7b4665a3f
input                0xd294f093
nonce                162
r                    0x6fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2bee
s                    0x0e804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583
to                   0x4a16A42407AA491564643E1dfc1fd50af29794eF
transactionIndex     0
v                    1
value                0
index                435
l1BlockNumber        12691036
l1Timestamp          1624460128
l1TxOrigin           null
queueIndex           null
queueOrigin          sequencer
rawTransaction       0xf86681a28084011cbbdc944a16a42407aa491564643e1dfc1fd50af29794ef8084d294f09338a06fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2beea00e804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583
txType               0
".trim()
        );
    }

    #[test]
    fn can_pretty_print_eip2930() {
        let s = r#"{
        "type": "0x1",
        "blockHash": "0x2b27fe2bbc8ce01ac7ae8bf74f793a197cf7edbe82727588811fa9a2c4776f81",
        "blockNumber": "0x12b1d",
        "from": "0x2b371c0262ceab27face32fbb5270ddc6aa01ba4",
        "gas": "0x6bdf",
        "gasPrice": "0x3b9aca00",
        "hash": "0xbddbb685774d8a3df036ed9fb920b48f876090a57e9e90ee60921e0510ef7090",
        "input": "0x9c0e3f7a0000000000000000000000000000000000000000000000000000000000000078000000000000000000000000000000000000000000000000000000000000002a",
        "nonce": "0x1c",
        "to": "0x8e730df7c70d33118d9e5f79ab81aed0be6f6635",
        "transactionIndex": "0x2",
        "value": "0x0",
        "v": "0x1",
        "r": "0x2a98c51c2782f664d3ce571fef0491b48f5ebbc5845fa513192e6e6b24ecdaa1",
        "s": "0x29b8e0c67aa9c11327e16556c591dc84a7aac2f6fc57c7f93901be8ee867aebc",
		"chainId": "0x66a",
		"accessList": [
			{ "address": "0x2b371c0262ceab27face32fbb5270ddc6aa01ba4", "storageKeys": ["0x1122334455667788990011223344556677889900112233445566778899001122", "0x0000000000000000000000000000000000000000000000000000000000000000"] },
			{ "address": "0x8e730df7c70d33118d9e5f79ab81aed0be6f6635", "storageKeys": [] }
		]
      }
        "#;

        let tx: Transaction = serde_json::from_str(s).unwrap();
        assert_eq!(tx.pretty().trim(),
                   r"
accessList           [
	0x2b371c0262CEAb27fAcE32FBB5270dDc6Aa01ba4 => [
		0x1122334455667788990011223344556677889900112233445566778899001122
		0x0000000000000000000000000000000000000000000000000000000000000000
	]
	0x8E730Df7C70D33118D9e5F79ab81aEd0bE6F6635 => []
]
blockHash            0x2b27fe2bbc8ce01ac7ae8bf74f793a197cf7edbe82727588811fa9a2c4776f81
blockNumber          76573
chainId              1642
from                 0x2b371c0262CEAb27fAcE32FBB5270dDc6Aa01ba4
gasLimit             27615
gasPrice             1000000000
hash                 0xbddbb685774d8a3df036ed9fb920b48f876090a57e9e90ee60921e0510ef7090
input                0x9c0e3f7a0000000000000000000000000000000000000000000000000000000000000078000000000000000000000000000000000000000000000000000000000000002a
nonce                28
r                    0x2a98c51c2782f664d3ce571fef0491b48f5ebbc5845fa513192e6e6b24ecdaa1
s                    0x29b8e0c67aa9c11327e16556c591dc84a7aac2f6fc57c7f93901be8ee867aebc
to                   0x8E730Df7C70D33118D9e5F79ab81aEd0bE6F6635
transactionIndex     2
type                 1
value                0
yParity              1
".trim()
        );
    }

    #[test]
    fn can_pretty_print_eip1559() {
        let s = r#"{
        "type": "0x2",
        "blockHash": "0x61abbe5e22738de0462046f5a5d6c4cd6bc1f3a6398e4457d5e293590e721125",
        "blockNumber": "0x7647",
        "from": "0xbaadf00d42264eeb3fafe6799d0b56cf55df0f00",
        "gas": "0x186a0",
        "hash": "0xa7231d4da0576fade5d3b9481f4cd52459ec59b9bbdbf4f60d6cd726b2a3a244",
        "input": "0x48600055323160015500",
        "nonce": "0x12c",
        "to": null,
        "transactionIndex": "0x41",
        "value": "0x0",
        "v": "0x1",
        "yParity": "0x1",
        "r": "0x396864e5f9132327defdb1449504252e1fa6bce73feb8cd6f348a342b198af34",
        "s": "0x44dbba72e6d3304104848277143252ee43627c82f02d1ef8e404e1bf97c70158",
        "gasPrice": "0x4a817c800",
        "maxFeePerGas": "0x4a817c800",
        "maxPriorityFeePerGas": "0x4a817c800",
        "chainId": "0x66a",
        "accessList": [
          {
            "address": "0xc141a9a7463e6c4716d9fc0c056c054f46bb2993",
            "storageKeys": [
              "0x0000000000000000000000000000000000000000000000000000000000000000"
            ]
          }
        ]
      }
"#;
        let tx: Transaction = serde_json::from_str(s).unwrap();
        assert_eq!(
            tx.pretty().trim(),
            r"
accessList           [
	0xC141a9A7463e6C4716d9FC0C056C054F46Bb2993 => [
		0x0000000000000000000000000000000000000000000000000000000000000000
	]
]
blockHash            0x61abbe5e22738de0462046f5a5d6c4cd6bc1f3a6398e4457d5e293590e721125
blockNumber          30279
chainId              1642
from                 0xBaaDF00d42264eEb3FAFe6799d0b56cf55DF0F00
gasLimit             100000
hash                 0xa7231d4da0576fade5d3b9481f4cd52459ec59b9bbdbf4f60d6cd726b2a3a244
input                0x48600055323160015500
maxFeePerGas         20000000000
maxPriorityFeePerGas 20000000000
nonce                300
r                    0x396864e5f9132327defdb1449504252e1fa6bce73feb8cd6f348a342b198af34
s                    0x44dbba72e6d3304104848277143252ee43627c82f02d1ef8e404e1bf97c70158
to                   
transactionIndex     65
type                 2
value                0
yParity              1
"
            .trim()
        );
    }

    #[test]
    fn can_pretty_print_eip4884() {
        let s = r#"{
		"blockHash": "0xfc2715ff196e23ae613ed6f837abd9035329a720a1f4e8dce3b0694c867ba052",
		"blockNumber": "0x2a1cb",
		"from": "0xad01b55d7c3448b8899862eb335fbb17075d8de2",
		"gas": "0x5208",
		"gasPrice": "0x1d1a94a201c",
		"maxFeePerGas": "0x1d1a94a201c",
		"maxPriorityFeePerGas": "0x1d1a94a201c",
		"maxFeePerBlobGas": "0x3e8",
		"hash": "0x5ceec39b631763ae0b45a8fb55c373f38b8fab308336ca1dc90ecd2b3cf06d00",
		"input": "0x",
		"nonce": "0x1b483",
		"to": "0x000000000000000000000000000000000000f1c1",
		"transactionIndex": "0x0",
		"value": "0x0",
		"type": "0x3",
		"accessList": [],
		"chainId": "0x1a1f0ff42",
		"blobVersionedHashes": [
		  "0x01a128c46fc61395706686d6284f83c6c86dfc15769b9363171ea9d8566e6e76"
		],
		"v": "0x0",
		"r": "0x343c6239323a81ef61293cb4a4d37b6df47fbf68114adb5dd41581151a077da1",
		"s": "0x48c21f6872feaf181d37cc4f9bbb356d3f10b352ceb38d1c3b190d749f95a11b",
		"yParity": "0x0"
	  }
"#;
        let tx: Transaction = serde_json::from_str(s).unwrap();
        assert_eq!(
            tx.pretty().trim(),
            r"
accessList           []
blobVersionedHashes  [
	0x01a128c46fc61395706686d6284f83c6c86dfc15769b9363171ea9d8566e6e76
]
blockHash            0xfc2715ff196e23ae613ed6f837abd9035329a720a1f4e8dce3b0694c867ba052
blockNumber          172491
chainId              7011893058
from                 0xAD01b55d7c3448B8899862eb335FBb17075d8DE2
gasLimit             21000
hash                 0x5ceec39b631763ae0b45a8fb55c373f38b8fab308336ca1dc90ecd2b3cf06d00
input                0x
maxFeePerBlobGas     1000
maxFeePerGas         2000000000028
maxPriorityFeePerGas 2000000000028
nonce                111747
r                    0x343c6239323a81ef61293cb4a4d37b6df47fbf68114adb5dd41581151a077da1
s                    0x48c21f6872feaf181d37cc4f9bbb356d3f10b352ceb38d1c3b190d749f95a11b
to                   0x000000000000000000000000000000000000f1C1
transactionIndex     0
type                 3
value                0
yParity              0
"
            .trim()
        );
    }

    #[test]
    fn print_block_w_txs() {
        let block = r#"{"number":"0x3","hash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","parentHash":"0x689c70c080ca22bc0e681694fa803c1aba16a69c8b6368fed5311d279eb9de90","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionsRoot":"0x7270c1c4440180f2bd5215809ee3d545df042b67329499e1ab97eb759d31610d","stateRoot":"0x29f32984517a7d25607da485b23cefabfd443751422ca7e603395e1de9bc8a4b","receiptsRoot":"0x056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2","miner":"0x0000000000000000000000000000000000000000","difficulty":"0x0","totalDifficulty":"0x0","extraData":"0x","size":"0x3e8","gasLimit":"0x6691b7","gasUsed":"0x5208","timestamp":"0x5ecedbb9","transactions":[{"hash":"0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067","nonce":"0x2","blockHash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","blockNumber":"0x3","transactionIndex":"0x0","from":"0xfdcedc3bfca10ecb0890337fbdd1977aba84807a","to":"0xdca8ce283150ab773bcbeb8d38289bdb5661de1e","value":"0x0","gas":"0x15f90","gasPrice":"0x4a817c800","input":"0x","v":"0x25","r":"0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88","s":"0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e"}],"uncles":[]}"#;
        let block: Block = serde_json::from_str(block).unwrap();
        let output ="\nblockHash            0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972
blockNumber          3
from                 0xFdCeDC3bFca10eCb0890337fbdD1977aba84807a
gas                  90000
gasPrice             20000000000
hash                 0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067
input                0x
nonce                2
r                    0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88
s                    0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e
to                   0xdca8ce283150AB773BCbeB8d38289bdB5661dE1e
transactionIndex     0
v                    0
value                0".to_string();
        let txs = match block.transactions() {
            BlockTransactions::Full(txs) => txs,
            _ => panic!("not full transactions"),
        };
        let generated = txs[0].pretty();
        assert_eq!(generated.as_str(), output.as_str());
    }

    #[test]
    fn uifmt_option_u64() {
        assert_eq!(None::<U64>.pretty(), "");
        assert_eq!(U64::from(100).pretty(), "100");
        assert_eq!(Some(U64::from(100)).pretty(), "100");
    }

    #[test]
    fn uifmt_option_h64() {
        assert_eq!(None::<B256>.pretty(), "");
        assert_eq!(
            B256::with_last_byte(100).pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
        assert_eq!(
            Some(B256::with_last_byte(100)).pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
    }

    #[test]
    fn uifmt_option_bytes() {
        assert_eq!(None::<Bytes>.pretty(), "");
        assert_eq!(
            Bytes::from_str("0x0000000000000000000000000000000000000000000000000000000000000064")
                .unwrap()
                .pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
        assert_eq!(
            Some(
                Bytes::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000000064"
                )
                .unwrap()
            )
            .pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
    }

    #[test]
    fn test_pretty_tx_attr() {
        let block = r#"{"number":"0x3","hash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","parentHash":"0x689c70c080ca22bc0e681694fa803c1aba16a69c8b6368fed5311d279eb9de90","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionsRoot":"0x7270c1c4440180f2bd5215809ee3d545df042b67329499e1ab97eb759d31610d","stateRoot":"0x29f32984517a7d25607da485b23cefabfd443751422ca7e603395e1de9bc8a4b","receiptsRoot":"0x056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2","miner":"0x0000000000000000000000000000000000000000","difficulty":"0x0","totalDifficulty":"0x0","extraData":"0x","size":"0x3e8","gasLimit":"0x6691b7","gasUsed":"0x5208","timestamp":"0x5ecedbb9","transactions":[{"hash":"0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067","nonce":"0x2","blockHash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","blockNumber":"0x3","transactionIndex":"0x0","from":"0xfdcedc3bfca10ecb0890337fbdd1977aba84807a","to":"0xdca8ce283150ab773bcbeb8d38289bdb5661de1e","value":"0x0","gas":"0x15f90","gasPrice":"0x4a817c800","input":"0x","v":"0x25","r":"0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88","s":"0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e"}],"uncles":[]}"#;
        let block: <FoundryNetwork as Network>::BlockResponse =
            serde_json::from_str(block).unwrap();
        let txs = match block.transactions() {
            BlockTransactions::Full(txes) => txes,
            _ => panic!("not full transactions"),
        };

        assert_eq!(None, get_pretty_tx_attr::<FoundryNetwork>(&txs[0], ""));
        assert_eq!(
            Some("3".to_string()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "blockNumber")
        );
        assert_eq!(
            Some("0xFdCeDC3bFca10eCb0890337fbdD1977aba84807a".to_string()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "from")
        );
        assert_eq!(Some("90000".to_string()), get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "gas"));
        assert_eq!(
            Some("20000000000".to_string()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "gasPrice")
        );
        assert_eq!(
            Some("0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067".to_string()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "hash")
        );
        assert_eq!(Some("0x".to_string()), get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "input"));
        assert_eq!(Some("2".to_string()), get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "nonce"));
        assert_eq!(
            Some("0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88".to_string()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "r")
        );
        assert_eq!(
            Some("0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e".to_string()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "s")
        );
        assert_eq!(
            Some("0xdca8ce283150AB773BCbeB8d38289bdB5661dE1e".into()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "to")
        );
        assert_eq!(
            Some("0".to_string()),
            get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "transactionIndex")
        );
        assert_eq!(Some("27".to_string()), get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "v"));
        assert_eq!(Some("0".to_string()), get_pretty_tx_attr::<FoundryNetwork>(&txs[0], "value"));
    }

    #[test]
    fn test_pretty_block_attr() {
        let json = serde_json::json!(
        {
            "baseFeePerGas": "0x7",
            "miner": "0x0000000000000000000000000000000000000001",
            "number": "0x1b4",
            "hash": "0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331",
            "parentHash": "0x9646252be9520f6e71339a8df9c55e4d7619deeb018d2a3f2d21fc165dde5eb5",
            "mixHash": "0x1010101010101010101010101010101010101010101010101010101010101010",
            "nonce": "0x0000000000000000",
            "sealFields": [
              "0xe04d296d2460cfb8472af2c5fd05b5a214109c25688d3704aed5484f9a7792f2",
              "0x0000000000000042"
            ],
            "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "logsBloom":  "0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331",
            "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "stateRoot": "0xd5855eb08b3387c0af375e9cdb6acfc05eb8f519e419b874b6ff2ffda7ed1dff",
            "difficulty": "0x27f07",
            "totalDifficulty": "0x27f07",
            "extraData": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "size": "0x27f07",
            "gasLimit": "0x9f759",
            "minGasPrice": "0x9f759",
            "gasUsed": "0x9f759",
            "timestamp": "0x54e34e8e",
            "transactions": [],
            "uncles": [],
          }
        );

        let block: <FoundryNetwork as Network>::BlockResponse =
            serde_json::from_value(json).unwrap();

        assert_eq!(None, get_pretty_block_attr::<FoundryNetwork>(&block, ""));
        assert_eq!(
            Some("7".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "baseFeePerGas")
        );
        assert_eq!(
            Some("163591".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "difficulty")
        );
        assert_eq!(
            Some("0x0000000000000000000000000000000000000000000000000000000000000000".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "extraData")
        );
        assert_eq!(
            Some("653145".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "gasLimit")
        );
        assert_eq!(
            Some("653145".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "gasUsed")
        );
        assert_eq!(
            Some("0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "hash")
        );
        assert_eq!(Some("0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331".to_string()), get_pretty_block_attr::<FoundryNetwork>(&block, "logsBloom"));
        assert_eq!(
            Some("0x0000000000000000000000000000000000000001".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "miner")
        );
        assert_eq!(
            Some("0x1010101010101010101010101010101010101010101010101010101010101010".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "mixHash")
        );
        assert_eq!(
            Some("0x0000000000000000".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "nonce")
        );
        assert_eq!(
            Some("436".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "number")
        );
        assert_eq!(
            Some("0x9646252be9520f6e71339a8df9c55e4d7619deeb018d2a3f2d21fc165dde5eb5".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "parentHash")
        );
        assert_eq!(
            Some("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "transactionsRoot")
        );
        assert_eq!(
            Some("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "receiptsRoot")
        );
        assert_eq!(
            Some("0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "sha3Uncles")
        );
        assert_eq!(
            Some("163591".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "size")
        );
        assert_eq!(
            Some("0xd5855eb08b3387c0af375e9cdb6acfc05eb8f519e419b874b6ff2ffda7ed1dff".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "stateRoot")
        );
        assert_eq!(
            Some("1424182926".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "timestamp")
        );
        assert_eq!(
            Some("163591".to_string()),
            get_pretty_block_attr::<FoundryNetwork>(&block, "totalDifficulty")
        );
    }

    #[test]
    fn test_receipt_other_fields_alignment() {
        let receipt_json = serde_json::json!(
        {
          "status": "0x1",
          "cumulativeGasUsed": "0x74e483",
          "logs": [],
          "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
          "type": "0x2",
          "transactionHash": "0x91181b0dca3b29aa136eeb2f536be5ce7b0aebc949be1c44b5509093c516097d",
          "transactionIndex": "0x10",
          "blockHash": "0x54bafb12e8cea9bb355fbf03a4ac49e42a2a1a80fa6cf4364b342e2de6432b5d",
          "blockNumber": "0x7b1ab93",
          "gasUsed": "0xc222",
          "effectiveGasPrice": "0x18961",
          "from": "0x2d815240a61731c75fa01b2793e1d3ed09f289d0",
          "to": "0x4200000000000000000000000000000000000000",
          "contractAddress": null,
          "l1BaseFeeScalar": "0x146b",
          "l1BlobBaseFee": "0x6a83078",
          "l1BlobBaseFeeScalar": "0xf79c5",
          "l1Fee": "0x51a9af7fd3",
          "l1GasPrice": "0x972fe4acc",
          "l1GasUsed": "0x640"
        });

        let receipt: AnyTransactionReceipt = serde_json::from_value(receipt_json).unwrap();
        let formatted = receipt.pretty();

        let expected = r#"
blockHash            0x54bafb12e8cea9bb355fbf03a4ac49e42a2a1a80fa6cf4364b342e2de6432b5d
blockNumber          129084307
contractAddress      
cumulativeGasUsed    7660675
effectiveGasPrice    100705
from                 0x2D815240A61731c75Fa01b2793E1D3eD09F289d0
gasUsed              49698
logs                 []
logsBloom            0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
root                 
status               1 (success)
transactionHash      0x91181b0dca3b29aa136eeb2f536be5ce7b0aebc949be1c44b5509093c516097d
transactionIndex     16
type                 2
blobGasPrice         
blobGasUsed          
to                   0x4200000000000000000000000000000000000000
l1BaseFeeScalar      5227
l1BlobBaseFee        111685752
l1BlobBaseFeeScalar  1014213
l1Fee                350739202003
l1GasPrice           40583973580
l1GasUsed            1600
"#;

        assert_eq!(formatted.trim(), expected.trim());
    }

    #[test]
    fn test_uifmt_for_signed_authorization() {
        let inner = Authorization {
            chain_id: U256::from(1),
            address: "0x000000000000000000000000000000000000dead".parse::<Address>().unwrap(),
            nonce: 42,
        };
        let signed_authorization =
            SignedAuthorization::new_unchecked(inner, 1, U256::from(20), U256::from(30));

        assert_eq!(
            signed_authorization.pretty(),
            r#"{recoveredAuthority: 0xf3eaBD0de6Ca1aE7fC4D81FfD6C9a40e5D5D7e30, signedAuthority: {"chainId":"0x1","address":"0x000000000000000000000000000000000000dead","nonce":"0x2a","yParity":"0x1","r":"0x14","s":"0x1e"}}"#
        );
    }

    #[test]
    fn can_pretty_print_tempo_tx() {
        let s = r#"{
            "type":"0x76",
            "chainId":"0xa5bd",
            "feeToken":"0x20c0000000000000000000000000000000000001",
            "maxPriorityFeePerGas":"0x0",
            "maxFeePerGas":"0x2cb417800",
            "gas":"0x2d178",
            "calls":[
                {
                    "data":null,
                    "input":"0x095ea7b3000000000000000000000000dec00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000989680",
                    "to":"0x20c0000000000000000000000000000000000000",
                    "value":"0x0"
                },
                {
                    "data":null,
                    "input":"0xf8856c0f00000000000000000000000020c000000000000000000000000000000000000000000000000000000000000020c00000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000989680000000000000000000000000000000000000000000000000000000000097d330",
                    "to":"0xdec0000000000000000000000000000000000000",
                    "value":"0x0"
                }
            ],
            "accessList":[],
            "nonceKey":"0x0",
            "nonce":"0x0",
            "feePayerSignature":null,
            "validBefore":null,
            "validAfter":null,
            "keyAuthorization":null,
            "aaAuthorizationList":[],
            "signature":{
                "pubKeyX":"0xaacc80b21e45fb11f349424dce3a2f23547f60c0ff2f8bcaede2a247545ce8dd",
                "pubKeyY":"0x87abf0dbb7a5c9507efae2e43833356651b45ac576c2e61cec4e9c0f41fcbf6e",
                "r":"0xcfd45c3b19745a42f80b134dcb02a8ba099a0e4e7be1984da54734aa81d8f29f",
                "s":"0x74bb9170ae6d25bd510c83fe35895ee5712efe13980a5edc8094c534e23af85e",
                "type":"webAuthn",
                "webauthnData":"0x7b98b7a8e6c68d7eac741a52e6fdae0560ce3c16ef5427ad46d7a54d0ed86dd41d000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a2238453071464a7a50585167546e645473643649456659457776323173516e626966374c4741776e4b43626b222c226f726967696e223a2268747470733a2f2f74656d706f2d6465782e76657263656c2e617070222c2263726f73734f726967696e223a66616c73657d"
            },
            "hash":"0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd",
            "blockHash":"0xc82b23589ceef5341ed307d33554714db6f9eefd4187a9ca8abc910a325b4689",
            "blockNumber":"0x321fde",
            "transactionIndex":"0x0",
            "from":"0x566ff0f4a6114f8072ecdc8a7a8a13d8d0c6b45f",
            "gasPrice":"0x2540be400"
        }"#;

        let tx: AnyRpcTransaction = serde_json::from_str(s).unwrap();

        assert_eq!(
            tx.pretty().trim(),
            r#"
blockHash            0xc82b23589ceef5341ed307d33554714db6f9eefd4187a9ca8abc910a325b4689
blockNumber          3284958
from                 0x566Ff0f4a6114F8072ecDC8A7A8A13d8d0C6B45F
transactionIndex     0
effectiveGasPrice    10000000000
hash                 0x6d6d8c102064e6dee44abad2024a8b1d37959230baab80e70efbf9b0c739c4fd
type                 118
aaAuthorizationList  []
accessList           []
calls                [{"data":null,"input":"0x095ea7b3000000000000000000000000dec00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000989680","to":"0x20c0000000000000000000000000000000000000","value":"0x0"},{"data":null,"input":"0xf8856c0f00000000000000000000000020c000000000000000000000000000000000000000000000000000000000000020c00000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000989680000000000000000000000000000000000000000000000000000000000097d330","to":"0xdec0000000000000000000000000000000000000","value":"0x0"}]
chainId              42429
feePayerSignature    null
feeToken             0x20C0000000000000000000000000000000000001
gas                  184696
gasPrice             10000000000
keyAuthorization     null
maxFeePerGas         12000000000
maxPriorityFeePerGas 0
nonce                0
nonceKey             0
signature            {"pubKeyX":"0xaacc80b21e45fb11f349424dce3a2f23547f60c0ff2f8bcaede2a247545ce8dd","pubKeyY":"0x87abf0dbb7a5c9507efae2e43833356651b45ac576c2e61cec4e9c0f41fcbf6e","r":"0xcfd45c3b19745a42f80b134dcb02a8ba099a0e4e7be1984da54734aa81d8f29f","s":"0x74bb9170ae6d25bd510c83fe35895ee5712efe13980a5edc8094c534e23af85e","type":"webAuthn","webauthnData":"0x7b98b7a8e6c68d7eac741a52e6fdae0560ce3c16ef5427ad46d7a54d0ed86dd41d000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a2238453071464a7a50585167546e645473643649456659457776323173516e626966374c4741776e4b43626b222c226f726967696e223a2268747470733a2f2f74656d706f2d6465782e76657263656c2e617070222c2263726f73734f726967696e223a66616c73657d"}
validAfter           null
validBefore          null
"#
                .trim()
        );
    }

    #[test]
    fn test_foundry_tx_receipt_uifmt() {
        use alloy_network::AnyTransactionReceipt;
        use foundry_primitives::FoundryTxReceipt;

        // Test UIfmt implementation for FoundryTxReceipt
        let s = r#"{"type":"0x2","status":"0x1","cumulativeGasUsed":"0x5208","logs":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionHash":"0x1234567890123456789012345678901234567890123456789012345678901234","transactionIndex":"0x0","blockHash":"0xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd","blockNumber":"0x1","gasUsed":"0x5208","effectiveGasPrice":"0x3b9aca00","from":"0x1234567890123456789012345678901234567890","to":"0x0987654321098765432109876543210987654321","contractAddress":null}"#;
        let any_receipt: AnyTransactionReceipt = serde_json::from_str(s).unwrap();
        let foundry_receipt = FoundryTxReceipt::try_from(any_receipt).unwrap();

        let pretty_output = foundry_receipt.pretty();

        // Check that essential fields are present in the output
        assert!(pretty_output.contains("blockHash"));
        assert!(pretty_output.contains("blockNumber"));
        assert!(pretty_output.contains("status"));
        assert!(pretty_output.contains("gasUsed"));
        assert!(pretty_output.contains("transactionHash"));
        assert!(pretty_output.contains("type"));

        // Verify the transaction hash appears in the output
        assert!(
            pretty_output
                .contains("0x1234567890123456789012345678901234567890123456789012345678901234")
        );

        // Verify status is pretty printed correctly (boolean true for successful transaction)
        assert!(pretty_output.contains("true"));
    }

    #[test]
    fn test_get_pretty_receipt_attr() {
        let receipt_json = serde_json::json!({
            "type": "0x2",
            "status": "0x1",
            "cumulativeGasUsed": "0x5208",
            "logs": [],
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "transactionHash": "0x1234567890123456789012345678901234567890123456789012345678901234",
            "transactionIndex": "0x0",
            "blockHash": "0xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd",
            "blockNumber": "0x1",
            "gasUsed": "0x5208",
            "effectiveGasPrice": "0x3b9aca00",
            "from": "0x1234567890123456789012345678901234567890",
            "to": "0x0987654321098765432109876543210987654321",
            "contractAddress": null
        });

        let receipt: <FoundryNetwork as Network>::ReceiptResponse =
            serde_json::from_value(receipt_json).unwrap();

        // Test basic receipt attributes
        assert_eq!(
            Some("0xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string()),
            get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "blockHash")
        );
        assert_eq!(
            Some("1".to_string()),
            get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "blockNumber")
        );
        assert_eq!(
            Some("0x1234567890123456789012345678901234567890123456789012345678901234".to_string()),
            get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "transactionHash")
        );
        assert_eq!(
            Some("21000".to_string()),
            get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "gasUsed")
        );
        assert_eq!(
            Some("true".to_string()),
            get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "status")
        );
        assert_eq!(
            Some("2".to_string()),
            get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "type")
        );
        assert_eq!(
            Some("[]".to_string()),
            get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "logs")
        );
        assert!(get_pretty_receipt_attr::<FoundryNetwork>(&receipt, "logsBloom").is_some());
    }
}
