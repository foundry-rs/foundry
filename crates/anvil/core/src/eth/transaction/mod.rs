//! transaction related data

use crate::eth::{
    receipt::Log,
    utils::{enveloped, to_revm_access_list},
};
use ethers_core::{
    types::{
        transaction::eip2930::{AccessList, AccessListItem},
        Address, Bloom, Bytes, Signature, SignatureError, TxHash, H256, U256, U64,
    },
    utils::{
        keccak256, rlp,
        rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream},
    },
};
use foundry_evm::trace::CallTraceArena;
use foundry_utils::types::ToAlloy;
use revm::{
    interpreter::InstructionResult,
    primitives::{CreateScheme, TransactTo, TxEnv},
};
use std::ops::Deref;

/// compatibility with `ethers-rs` types
mod ethers_compat;

/// The signature used to bypass signing via the `eth_sendUnsignedTransaction` cheat RPC
#[cfg(feature = "impersonated-tx")]
pub const IMPERSONATED_SIGNATURE: Signature =
    Signature { r: U256([0, 0, 0, 0]), s: U256([0, 0, 0, 0]), v: 0 };

/// Container type for various Ethereum transaction requests
///
/// Its variants correspond to specific allowed transactions:
/// 1. Legacy (pre-EIP2718) [`LegacyTransactionRequest`]
/// 2. EIP2930 (state access lists) [`EIP2930TransactionRequest`]
/// 3. EIP1559 [`EIP1559TransactionRequest`]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TypedTransactionRequest {
    Legacy(LegacyTransactionRequest),
    EIP2930(EIP2930TransactionRequest),
    EIP1559(EIP1559TransactionRequest),
}

/// Represents _all_ transaction requests received from RPC
#[derive(Clone, Debug, PartialEq, Eq, Default, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct EthTransactionRequest {
    /// from address
    pub from: Option<Address>,
    /// to address
    pub to: Option<Address>,
    /// legacy, gas Price
    #[cfg_attr(feature = "serde", serde(default))]
    pub gas_price: Option<U256>,
    /// max base fee per gas sender is willing to pay
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_fee_per_gas: Option<U256>,
    /// miner tip
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_priority_fee_per_gas: Option<U256>,
    /// gas
    pub gas: Option<U256>,
    /// value of th tx in wei
    pub value: Option<U256>,
    /// Any additional data sent
    pub data: Option<Bytes>,
    /// Transaction nonce
    pub nonce: Option<U256>,
    /// chain id
    #[cfg_attr(feature = "serde", serde(default))]
    pub chain_id: Option<U64>,
    /// warm storage access pre-payment
    #[cfg_attr(feature = "serde", serde(default))]
    pub access_list: Option<Vec<AccessListItem>>,
    /// EIP-2718 type
    #[cfg_attr(feature = "serde", serde(rename = "type"))]
    pub transaction_type: Option<U256>,
}

// == impl EthTransactionRequest ==

impl EthTransactionRequest {
    /// Converts the request into a [TypedTransactionRequest]
    pub fn into_typed_request(self) -> Option<TypedTransactionRequest> {
        let EthTransactionRequest {
            to,
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas,
            value,
            data,
            nonce,
            mut access_list,
            chain_id,
            transaction_type,
            ..
        } = self;
        let chain_id = chain_id.map(|id| id.as_u64());
        let transaction_type = transaction_type.map(|id| id.as_u64());
        match (
            transaction_type,
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            access_list.take(),
        ) {
            // legacy transaction
            (Some(0), _, None, None, None) | (None, Some(_), None, None, None) => {
                Some(TypedTransactionRequest::Legacy(LegacyTransactionRequest {
                    nonce: nonce.unwrap_or(U256::zero()),
                    gas_price: gas_price.unwrap_or_default(),
                    gas_limit: gas.unwrap_or_default(),
                    value: value.unwrap_or(U256::zero()),
                    input: data.unwrap_or_default(),
                    kind: match to {
                        Some(to) => TransactionKind::Call(to),
                        None => TransactionKind::Create,
                    },
                    chain_id,
                }))
            }
            // EIP2930
            (Some(1), _, None, None, _) | (None, _, None, None, Some(_)) => {
                Some(TypedTransactionRequest::EIP2930(EIP2930TransactionRequest {
                    nonce: nonce.unwrap_or(U256::zero()),
                    gas_price: gas_price.unwrap_or_default(),
                    gas_limit: gas.unwrap_or_default(),
                    value: value.unwrap_or(U256::zero()),
                    input: data.unwrap_or_default(),
                    kind: match to {
                        Some(to) => TransactionKind::Call(to),
                        None => TransactionKind::Create,
                    },
                    chain_id: chain_id.unwrap_or_default(),
                    access_list: access_list.unwrap_or_default(),
                }))
            }
            // EIP1559
            (Some(2), None, _, _, _) |
            (None, None, Some(_), _, _) |
            (None, None, _, Some(_), _) |
            (None, None, None, None, None) => {
                // Empty fields fall back to the canonical transaction schema.
                Some(TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
                    nonce: nonce.unwrap_or(U256::zero()),
                    max_fee_per_gas: max_fee_per_gas.unwrap_or_default(),
                    max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or(U256::zero()),
                    gas_limit: gas.unwrap_or_default(),
                    value: value.unwrap_or(U256::zero()),
                    input: data.unwrap_or_default(),
                    kind: match to {
                        Some(to) => TransactionKind::Call(to),
                        None => TransactionKind::Create,
                    },
                    chain_id: chain_id.unwrap_or_default(),
                    access_list: access_list.unwrap_or_default(),
                }))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransactionKind {
    Call(Address),
    Create,
}

// == impl TransactionKind ==

impl TransactionKind {
    /// If this transaction is a call this returns the address of the callee
    pub fn as_call(&self) -> Option<&Address> {
        match self {
            TransactionKind::Call(to) => Some(to),
            TransactionKind::Create => None,
        }
    }
}

impl Encodable for TransactionKind {
    fn rlp_append(&self, s: &mut RlpStream) {
        match self {
            TransactionKind::Call(address) => {
                s.encoder().encode_value(&address[..]);
            }
            TransactionKind::Create => s.encoder().encode_value(&[]),
        }
    }
}

impl Decodable for TransactionKind {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if rlp.is_empty() {
            if rlp.is_data() {
                Ok(TransactionKind::Create)
            } else {
                Err(DecoderError::RlpExpectedToBeData)
            }
        } else {
            Ok(TransactionKind::Call(rlp.as_val()?))
        }
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Encodable for TransactionKind {
    fn length(&self) -> usize {
        match self {
            TransactionKind::Call(to) => to.length(),
            TransactionKind::Create => ([]).length(),
        }
    }
    fn encode(&self, out: &mut dyn open_fastrlp::BufMut) {
        match self {
            TransactionKind::Call(to) => to.encode(out),
            TransactionKind::Create => ([]).encode(out),
        }
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Decodable for TransactionKind {
    fn decode(buf: &mut &[u8]) -> Result<Self, open_fastrlp::DecodeError> {
        use bytes::Buf;

        if let Some(&first) = buf.first() {
            if first == 0x80 {
                buf.advance(1);
                Ok(TransactionKind::Create)
            } else {
                let addr = <Address as open_fastrlp::Decodable>::decode(buf)?;
                Ok(TransactionKind::Call(addr))
            }
        } else {
            Err(open_fastrlp::DecodeError::InputTooShort)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
pub struct EIP2930TransactionRequest {
    pub chain_id: u64,
    pub nonce: U256,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub kind: TransactionKind,
    pub value: U256,
    pub input: Bytes,
    pub access_list: Vec<AccessListItem>,
}

impl EIP2930TransactionRequest {
    pub fn hash(&self) -> H256 {
        let encoded = rlp::encode(self);
        let mut out = vec![0; 1 + encoded.len()];
        out[0] = 1;
        out[1..].copy_from_slice(&encoded);
        H256::from_slice(keccak256(&out).as_slice())
    }
}

impl From<EIP2930Transaction> for EIP2930TransactionRequest {
    fn from(tx: EIP2930Transaction) -> Self {
        Self {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_price: tx.gas_price,
            gas_limit: tx.gas_limit,
            kind: tx.kind,
            value: tx.value,
            input: tx.input,
            access_list: tx.access_list.0,
        }
    }
}

impl Encodable for EIP2930TransactionRequest {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(8);
        s.append(&self.chain_id);
        s.append(&self.nonce);
        s.append(&self.gas_price);
        s.append(&self.gas_limit);
        s.append(&self.kind);
        s.append(&self.value);
        s.append(&self.input.as_ref());
        s.append_list(&self.access_list);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyTransactionRequest {
    pub nonce: U256,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub kind: TransactionKind,
    pub value: U256,
    pub input: Bytes,
    pub chain_id: Option<u64>,
}

// == impl LegacyTransactionRequest ==

impl LegacyTransactionRequest {
    pub fn hash(&self) -> H256 {
        H256::from_slice(keccak256(&rlp::encode(self)).as_slice())
    }
}

impl From<LegacyTransaction> for LegacyTransactionRequest {
    fn from(tx: LegacyTransaction) -> Self {
        let chain_id = tx.chain_id();
        Self {
            nonce: tx.nonce,
            gas_price: tx.gas_price,
            gas_limit: tx.gas_limit,
            kind: tx.kind,
            value: tx.value,
            input: tx.input,
            chain_id,
        }
    }
}

impl Encodable for LegacyTransactionRequest {
    fn rlp_append(&self, s: &mut RlpStream) {
        if let Some(chain_id) = self.chain_id {
            s.begin_list(9);
            s.append(&self.nonce);
            s.append(&self.gas_price);
            s.append(&self.gas_limit);
            s.append(&self.kind);
            s.append(&self.value);
            s.append(&self.input.as_ref());
            s.append(&chain_id);
            s.append(&0u8);
            s.append(&0u8);
        } else {
            s.begin_list(6);
            s.append(&self.nonce);
            s.append(&self.gas_price);
            s.append(&self.gas_limit);
            s.append(&self.kind);
            s.append(&self.value);
            s.append(&self.input.as_ref());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
pub struct EIP1559TransactionRequest {
    pub chain_id: u64,
    pub nonce: U256,
    pub max_priority_fee_per_gas: U256,
    pub max_fee_per_gas: U256,
    pub gas_limit: U256,
    pub kind: TransactionKind,
    pub value: U256,
    pub input: Bytes,
    pub access_list: Vec<AccessListItem>,
}

// == impl EIP1559TransactionRequest ==

impl EIP1559TransactionRequest {
    pub fn hash(&self) -> H256 {
        let encoded = rlp::encode(self);
        let mut out = vec![0; 1 + encoded.len()];
        out[0] = 2;
        out[1..].copy_from_slice(&encoded);
        H256::from_slice(keccak256(&out).as_slice())
    }
}

impl From<EIP1559Transaction> for EIP1559TransactionRequest {
    fn from(t: EIP1559Transaction) -> Self {
        Self {
            chain_id: t.chain_id,
            nonce: t.nonce,
            max_priority_fee_per_gas: t.max_priority_fee_per_gas,
            max_fee_per_gas: t.max_fee_per_gas,
            gas_limit: t.gas_limit,
            kind: t.kind,
            value: t.value,
            input: t.input,
            access_list: t.access_list.0,
        }
    }
}

impl Encodable for EIP1559TransactionRequest {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(9);
        s.append(&self.chain_id);
        s.append(&self.nonce);
        s.append(&self.max_priority_fee_per_gas);
        s.append(&self.max_fee_per_gas);
        s.append(&self.gas_limit);
        s.append(&self.kind);
        s.append(&self.value);
        s.append(&self.input.as_ref());
        s.append_list(&self.access_list);
    }
}

/// A wrapper for `TypedTransaction` that allows impersonating accounts.
///
/// This is a helper that carries the `impersonated` sender so that the right hash
/// [TypedTransaction::impersonated_hash] can be created.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaybeImpersonatedTransaction {
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub transaction: TypedTransaction,
    #[cfg_attr(feature = "serde", serde(skip))]
    pub impersonated_sender: Option<Address>,
}

// === impl MaybeImpersonatedTransaction ===

impl MaybeImpersonatedTransaction {
    /// Creates a new wrapper for the given transaction
    pub fn new(transaction: TypedTransaction) -> Self {
        Self { transaction, impersonated_sender: None }
    }

    /// Creates a new impersonated transaction wrapper using the given sender
    pub fn impersonated(transaction: TypedTransaction, impersonated_sender: Address) -> Self {
        Self { transaction, impersonated_sender: Some(impersonated_sender) }
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    ///
    /// Note: this is feature gated so it does not conflict with the `Deref`ed
    /// [TypedTransaction::recover] function by default.
    #[cfg(feature = "impersonated-tx")]
    pub fn recover(&self) -> Result<Address, SignatureError> {
        if let Some(sender) = self.impersonated_sender {
            return Ok(sender)
        }
        self.transaction.recover()
    }

    /// Returns the hash of the transaction
    ///
    /// Note: this is feature gated so it does not conflict with the `Deref`ed
    /// [TypedTransaction::hash] function by default.
    #[cfg(feature = "impersonated-tx")]
    pub fn hash(&self) -> H256 {
        if self.transaction.is_impersonated() {
            if let Some(sender) = self.impersonated_sender {
                return self.transaction.impersonated_hash(sender)
            }
        }
        self.transaction.hash()
    }
}

impl Encodable for MaybeImpersonatedTransaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        self.transaction.rlp_append(s)
    }
}

impl From<MaybeImpersonatedTransaction> for TypedTransaction {
    fn from(value: MaybeImpersonatedTransaction) -> Self {
        value.transaction
    }
}

impl From<TypedTransaction> for MaybeImpersonatedTransaction {
    fn from(value: TypedTransaction) -> Self {
        MaybeImpersonatedTransaction::new(value)
    }
}

impl Decodable for MaybeImpersonatedTransaction {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let transaction = TypedTransaction::decode(rlp)?;
        Ok(Self { transaction, impersonated_sender: None })
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Encodable for MaybeImpersonatedTransaction {
    fn encode(&self, out: &mut dyn open_fastrlp::BufMut) {
        self.transaction.encode(out)
    }
    fn length(&self) -> usize {
        self.transaction.length()
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Decodable for MaybeImpersonatedTransaction {
    fn decode(buf: &mut &[u8]) -> Result<Self, open_fastrlp::DecodeError> {
        Ok(Self { transaction: open_fastrlp::Decodable::decode(buf)?, impersonated_sender: None })
    }
}

impl AsRef<TypedTransaction> for MaybeImpersonatedTransaction {
    fn as_ref(&self) -> &TypedTransaction {
        &self.transaction
    }
}

impl Deref for MaybeImpersonatedTransaction {
    type Target = TypedTransaction;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TypedTransaction {
    /// Legacy transaction type
    Legacy(LegacyTransaction),
    /// EIP-2930 transaction
    EIP2930(EIP2930Transaction),
    /// EIP-1559 transaction
    EIP1559(EIP1559Transaction),
}

// == impl TypedTransaction ==

impl TypedTransaction {
    /// Returns true if the transaction uses dynamic fees: EIP1559
    pub fn is_dynamic_fee(&self) -> bool {
        matches!(self, TypedTransaction::EIP1559(_))
    }

    pub fn gas_price(&self) -> U256 {
        match self {
            TypedTransaction::Legacy(tx) => tx.gas_price,
            TypedTransaction::EIP2930(tx) => tx.gas_price,
            TypedTransaction::EIP1559(tx) => tx.max_fee_per_gas,
        }
    }

    pub fn gas_limit(&self) -> U256 {
        match self {
            TypedTransaction::Legacy(tx) => tx.gas_limit,
            TypedTransaction::EIP2930(tx) => tx.gas_limit,
            TypedTransaction::EIP1559(tx) => tx.gas_limit,
        }
    }

    pub fn value(&self) -> U256 {
        match self {
            TypedTransaction::Legacy(tx) => tx.value,
            TypedTransaction::EIP2930(tx) => tx.value,
            TypedTransaction::EIP1559(tx) => tx.value,
        }
    }

    pub fn data(&self) -> &Bytes {
        match self {
            TypedTransaction::Legacy(tx) => &tx.input,
            TypedTransaction::EIP2930(tx) => &tx.input,
            TypedTransaction::EIP1559(tx) => &tx.input,
        }
    }

    /// Returns the transaction type
    pub fn r#type(&self) -> Option<u8> {
        match self {
            TypedTransaction::Legacy(_) => None,
            TypedTransaction::EIP2930(_) => Some(1),
            TypedTransaction::EIP1559(_) => Some(2),
        }
    }

    /// Max cost of the transaction
    pub fn max_cost(&self) -> U256 {
        self.gas_limit().saturating_mul(self.gas_price())
    }

    /// Returns a helper type that contains commonly used values as fields
    pub fn essentials(&self) -> TransactionEssentials {
        match self {
            TypedTransaction::Legacy(t) => TransactionEssentials {
                kind: t.kind,
                input: t.input.clone(),
                nonce: t.nonce,
                gas_limit: t.gas_limit,
                gas_price: Some(t.gas_price),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                value: t.value,
                chain_id: t.chain_id(),
                access_list: Default::default(),
            },
            TypedTransaction::EIP2930(t) => TransactionEssentials {
                kind: t.kind,
                input: t.input.clone(),
                nonce: t.nonce,
                gas_limit: t.gas_limit,
                gas_price: Some(t.gas_price),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                value: t.value,
                chain_id: Some(t.chain_id),
                access_list: t.access_list.clone(),
            },
            TypedTransaction::EIP1559(t) => TransactionEssentials {
                kind: t.kind,
                input: t.input.clone(),
                nonce: t.nonce,
                gas_limit: t.gas_limit,
                gas_price: None,
                max_fee_per_gas: Some(t.max_fee_per_gas),
                max_priority_fee_per_gas: Some(t.max_priority_fee_per_gas),
                value: t.value,
                chain_id: Some(t.chain_id),
                access_list: t.access_list.clone(),
            },
        }
    }

    pub fn nonce(&self) -> &U256 {
        match self {
            TypedTransaction::Legacy(t) => t.nonce(),
            TypedTransaction::EIP2930(t) => t.nonce(),
            TypedTransaction::EIP1559(t) => t.nonce(),
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        match self {
            TypedTransaction::Legacy(t) => t.chain_id(),
            TypedTransaction::EIP2930(t) => Some(t.chain_id),
            TypedTransaction::EIP1559(t) => Some(t.chain_id),
        }
    }

    pub fn as_legacy(&self) -> Option<&LegacyTransaction> {
        match self {
            TypedTransaction::Legacy(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns true whether this tx is a legacy transaction
    pub fn is_legacy(&self) -> bool {
        matches!(self, TypedTransaction::Legacy(_))
    }

    /// Returns true whether this tx is a EIP1559 transaction
    pub fn is_eip1559(&self) -> bool {
        matches!(self, TypedTransaction::EIP1559(_))
    }

    /// Returns the hash of the transaction.
    ///
    /// Note: If this transaction has the Impersonated signature then this returns a modified unique
    /// hash. This allows us to treat impersonated transactions as unique.
    pub fn hash(&self) -> H256 {
        match self {
            TypedTransaction::Legacy(t) => t.hash(),
            TypedTransaction::EIP2930(t) => t.hash(),
            TypedTransaction::EIP1559(t) => t.hash(),
        }
    }

    /// Returns true if the transaction was impersonated (using the impersonate Signature)
    #[cfg(feature = "impersonated-tx")]
    pub fn is_impersonated(&self) -> bool {
        self.signature() == IMPERSONATED_SIGNATURE
    }

    /// Returns the hash if the transaction is impersonated (using a fake signature)
    ///
    /// This appends the `address` before hashing it
    #[cfg(feature = "impersonated-tx")]
    pub fn impersonated_hash(&self, sender: Address) -> H256 {
        let mut bytes = rlp::encode(self);
        bytes.extend_from_slice(sender.as_ref());
        H256::from_slice(keccak256(&bytes).as_slice())
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, SignatureError> {
        match self {
            TypedTransaction::Legacy(tx) => tx.recover(),
            TypedTransaction::EIP2930(tx) => tx.recover(),
            TypedTransaction::EIP1559(tx) => tx.recover(),
        }
    }

    /// Returns what kind of transaction this is
    pub fn kind(&self) -> &TransactionKind {
        match self {
            TypedTransaction::Legacy(tx) => &tx.kind,
            TypedTransaction::EIP2930(tx) => &tx.kind,
            TypedTransaction::EIP1559(tx) => &tx.kind,
        }
    }

    /// Returns the callee if this transaction is a call
    pub fn to(&self) -> Option<&Address> {
        self.kind().as_call()
    }

    /// Returns the Signature of the transaction
    pub fn signature(&self) -> Signature {
        match self {
            TypedTransaction::Legacy(tx) => tx.signature,
            TypedTransaction::EIP2930(tx) => {
                let v = tx.odd_y_parity as u8;
                let r = U256::from_big_endian(&tx.r[..]);
                let s = U256::from_big_endian(&tx.s[..]);
                Signature { r, s, v: v.into() }
            }
            TypedTransaction::EIP1559(tx) => {
                let v = tx.odd_y_parity as u8;
                let r = U256::from_big_endian(&tx.r[..]);
                let s = U256::from_big_endian(&tx.s[..]);
                Signature { r, s, v: v.into() }
            }
        }
    }
}

impl Encodable for TypedTransaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        match self {
            TypedTransaction::Legacy(tx) => tx.rlp_append(s),
            TypedTransaction::EIP2930(tx) => enveloped(1, tx, s),
            TypedTransaction::EIP1559(tx) => enveloped(2, tx, s),
        }
    }
}

impl Decodable for TypedTransaction {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let data = rlp.data()?;
        let first = *data.first().ok_or(DecoderError::Custom("empty slice"))?;
        if rlp.is_list() {
            return Ok(TypedTransaction::Legacy(rlp.as_val()?))
        }
        let s = data.get(1..).ok_or(DecoderError::Custom("no tx body"))?;
        if first == 0x01 {
            return rlp::decode(s).map(TypedTransaction::EIP2930)
        }
        if first == 0x02 {
            return rlp::decode(s).map(TypedTransaction::EIP1559)
        }
        Err(DecoderError::Custom("invalid tx type"))
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Encodable for TypedTransaction {
    fn encode(&self, out: &mut dyn open_fastrlp::BufMut) {
        match self {
            TypedTransaction::Legacy(tx) => tx.encode(out),
            tx => {
                let payload_len = match tx {
                    TypedTransaction::EIP2930(tx) => tx.length() + 1,
                    TypedTransaction::EIP1559(tx) => tx.length() + 1,
                    _ => unreachable!("legacy tx length already matched"),
                };

                match tx {
                    TypedTransaction::EIP2930(tx) => {
                        let tx_string_header =
                            open_fastrlp::Header { list: false, payload_length: payload_len };

                        tx_string_header.encode(out);
                        out.put_u8(0x01);
                        tx.encode(out);
                    }
                    TypedTransaction::EIP1559(tx) => {
                        let tx_string_header =
                            open_fastrlp::Header { list: false, payload_length: payload_len };

                        tx_string_header.encode(out);
                        out.put_u8(0x02);
                        tx.encode(out);
                    }
                    _ => unreachable!("legacy tx encode already matched"),
                }
            }
        }
    }
    fn length(&self) -> usize {
        match self {
            TypedTransaction::Legacy(tx) => tx.length(),
            tx => {
                let payload_len = match tx {
                    TypedTransaction::EIP2930(tx) => tx.length() + 1,
                    TypedTransaction::EIP1559(tx) => tx.length() + 1,
                    _ => unreachable!("legacy tx length already matched"),
                };
                // we include a string header for signed types txs, so include the length here
                payload_len + open_fastrlp::length_of_length(payload_len)
            }
        }
    }
}

#[cfg(feature = "fastrlp")]
impl open_fastrlp::Decodable for TypedTransaction {
    fn decode(buf: &mut &[u8]) -> Result<Self, open_fastrlp::DecodeError> {
        use bytes::Buf;
        use std::cmp::Ordering;

        let first = *buf.first().ok_or(open_fastrlp::DecodeError::Custom("empty slice"))?;

        // a signed transaction is either encoded as a string (non legacy) or a list (legacy).
        // We should not consume the buffer if we are decoding a legacy transaction, so let's
        // check if the first byte is between 0x80 and 0xbf.
        match first.cmp(&open_fastrlp::EMPTY_LIST_CODE) {
            Ordering::Less => {
                // strip out the string header
                // NOTE: typed transaction encodings either contain a "rlp header" which contains
                // the type of the payload and its length, or they do not contain a header and
                // start with the tx type byte.
                //
                // This line works for both types of encodings because byte slices starting with
                // 0x01 and 0x02 return a Header { list: false, payload_length: 1 } when input to
                // Header::decode.
                // If the encoding includes a header, the header will be properly decoded and
                // consumed.
                // Otherwise, header decoding will succeed but nothing is consumed.
                let _header = open_fastrlp::Header::decode(buf)?;
                let tx_type = *buf.first().ok_or(open_fastrlp::DecodeError::Custom(
                    "typed tx cannot be decoded from an empty slice",
                ))?;
                if tx_type == 0x01 {
                    buf.advance(1);
                    <EIP2930Transaction as open_fastrlp::Decodable>::decode(buf)
                        .map(TypedTransaction::EIP2930)
                } else if tx_type == 0x02 {
                    buf.advance(1);
                    <EIP1559Transaction as open_fastrlp::Decodable>::decode(buf)
                        .map(TypedTransaction::EIP1559)
                } else {
                    Err(open_fastrlp::DecodeError::Custom("invalid tx type"))
                }
            }
            Ordering::Equal => Err(open_fastrlp::DecodeError::Custom(
                "an empty list is not a valid transaction encoding",
            )),
            Ordering::Greater => <LegacyTransaction as open_fastrlp::Decodable>::decode(buf)
                .map(TypedTransaction::Legacy),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LegacyTransaction {
    pub nonce: U256,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub kind: TransactionKind,
    pub value: U256,
    pub input: Bytes,
    pub signature: Signature,
}

impl LegacyTransaction {
    pub fn nonce(&self) -> &U256 {
        &self.nonce
    }

    pub fn hash(&self) -> H256 {
        H256::from_slice(keccak256(&rlp::encode(self)).as_slice())
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, SignatureError> {
        self.signature.recover(LegacyTransactionRequest::from(self.clone()).hash())
    }

    pub fn chain_id(&self) -> Option<u64> {
        if self.signature.v > 36 {
            Some((self.signature.v - 35) / 2)
        } else {
            None
        }
    }

    /// See <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
    /// > If you do, then the v of the signature MUST be set to {0,1} + CHAIN_ID * 2 + 35 where
    /// > {0,1} is the parity of the y value of the curve point for which r is the x-value in the
    /// > secp256k1 signing process.
    pub fn meets_eip155(&self, chain_id: u64) -> bool {
        let double_chain_id = chain_id.saturating_mul(2);
        let v = self.signature.v;
        v == double_chain_id + 35 || v == double_chain_id + 36
    }
}

impl Encodable for LegacyTransaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(9);
        s.append(&self.nonce);
        s.append(&self.gas_price);
        s.append(&self.gas_limit);
        s.append(&self.kind);
        s.append(&self.value);
        s.append(&self.input.as_ref());
        s.append(&self.signature.v);
        s.append(&self.signature.r);
        s.append(&self.signature.s);
    }
}

impl Decodable for LegacyTransaction {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if rlp.item_count()? != 9 {
            return Err(DecoderError::RlpIncorrectListLen)
        }

        let v = rlp.val_at(6)?;
        let r = rlp.val_at::<U256>(7)?;
        let s = rlp.val_at::<U256>(8)?;

        Ok(Self {
            nonce: rlp.val_at(0)?,
            gas_price: rlp.val_at(1)?,
            gas_limit: rlp.val_at(2)?,
            kind: rlp.val_at(3)?,
            value: rlp.val_at(4)?,
            input: rlp.val_at::<Vec<u8>>(5)?.into(),
            signature: Signature { v, r, s },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EIP2930Transaction {
    pub chain_id: u64,
    pub nonce: U256,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub kind: TransactionKind,
    pub value: U256,
    pub input: Bytes,
    pub access_list: AccessList,
    pub odd_y_parity: bool,
    pub r: H256,
    pub s: H256,
}

impl EIP2930Transaction {
    pub fn nonce(&self) -> &U256 {
        &self.nonce
    }

    pub fn hash(&self) -> H256 {
        let encoded = rlp::encode(self);
        let mut out = vec![0; 1 + encoded.len()];
        out[0] = 1;
        out[1..].copy_from_slice(&encoded);
        H256::from_slice(keccak256(&out).as_slice())
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, SignatureError> {
        let mut sig = [0u8; 65];
        sig[0..32].copy_from_slice(&self.r[..]);
        sig[32..64].copy_from_slice(&self.s[..]);
        sig[64] = self.odd_y_parity as u8;
        let signature = Signature::try_from(&sig[..])?;
        signature.recover(EIP2930TransactionRequest::from(self.clone()).hash())
    }
}

impl Encodable for EIP2930Transaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(11);
        s.append(&self.chain_id);
        s.append(&self.nonce);
        s.append(&self.gas_price);
        s.append(&self.gas_limit);
        s.append(&self.kind);
        s.append(&self.value);
        s.append(&self.input.as_ref());
        s.append(&self.access_list);
        s.append(&self.odd_y_parity);
        s.append(&U256::from_big_endian(&self.r[..]));
        s.append(&U256::from_big_endian(&self.s[..]));
    }
}

impl Decodable for EIP2930Transaction {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if rlp.item_count()? != 11 {
            return Err(DecoderError::RlpIncorrectListLen)
        }

        Ok(Self {
            chain_id: rlp.val_at(0)?,
            nonce: rlp.val_at(1)?,
            gas_price: rlp.val_at(2)?,
            gas_limit: rlp.val_at(3)?,
            kind: rlp.val_at(4)?,
            value: rlp.val_at(5)?,
            input: rlp.val_at::<Vec<u8>>(6)?.into(),
            access_list: rlp.val_at(7)?,
            odd_y_parity: rlp.val_at(8)?,
            r: {
                let mut rarr = [0u8; 32];
                rlp.val_at::<U256>(9)?.to_big_endian(&mut rarr);
                H256::from(rarr)
            },
            s: {
                let mut sarr = [0u8; 32];
                rlp.val_at::<U256>(10)?.to_big_endian(&mut sarr);
                H256::from(sarr)
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "fastrlp", derive(open_fastrlp::RlpEncodable, open_fastrlp::RlpDecodable))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EIP1559Transaction {
    pub chain_id: u64,
    pub nonce: U256,
    pub max_priority_fee_per_gas: U256,
    pub max_fee_per_gas: U256,
    pub gas_limit: U256,
    pub kind: TransactionKind,
    pub value: U256,
    pub input: Bytes,
    pub access_list: AccessList,
    pub odd_y_parity: bool,
    pub r: H256,
    pub s: H256,
}

impl EIP1559Transaction {
    pub fn nonce(&self) -> &U256 {
        &self.nonce
    }

    pub fn hash(&self) -> H256 {
        let encoded = rlp::encode(self);
        let mut out = vec![0; 1 + encoded.len()];
        out[0] = 2;
        out[1..].copy_from_slice(&encoded);
        H256::from_slice(keccak256(&out).as_slice())
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, SignatureError> {
        let mut sig = [0u8; 65];
        sig[0..32].copy_from_slice(&self.r[..]);
        sig[32..64].copy_from_slice(&self.s[..]);
        sig[64] = self.odd_y_parity as u8;
        let signature = Signature::try_from(&sig[..])?;
        signature.recover(EIP1559TransactionRequest::from(self.clone()).hash())
    }
}

impl Encodable for EIP1559Transaction {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(12);
        s.append(&self.chain_id);
        s.append(&self.nonce);
        s.append(&self.max_priority_fee_per_gas);
        s.append(&self.max_fee_per_gas);
        s.append(&self.gas_limit);
        s.append(&self.kind);
        s.append(&self.value);
        s.append(&self.input.as_ref());
        s.append(&self.access_list);
        s.append(&self.odd_y_parity);
        s.append(&U256::from_big_endian(&self.r[..]));
        s.append(&U256::from_big_endian(&self.s[..]));
    }
}

impl Decodable for EIP1559Transaction {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        if rlp.item_count()? != 12 {
            return Err(DecoderError::RlpIncorrectListLen)
        }

        Ok(Self {
            chain_id: rlp.val_at(0)?,
            nonce: rlp.val_at(1)?,
            max_priority_fee_per_gas: rlp.val_at(2)?,
            max_fee_per_gas: rlp.val_at(3)?,
            gas_limit: rlp.val_at(4)?,
            kind: rlp.val_at(5)?,
            value: rlp.val_at(6)?,
            input: rlp.val_at::<Vec<u8>>(7)?.into(),
            access_list: rlp.val_at(8)?,
            odd_y_parity: rlp.val_at(9)?,
            r: {
                let mut rarr = [0u8; 32];
                rlp.val_at::<U256>(10)?.to_big_endian(&mut rarr);
                H256::from(rarr)
            },
            s: {
                let mut sarr = [0u8; 32];
                rlp.val_at::<U256>(11)?.to_big_endian(&mut sarr);
                H256::from(sarr)
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionEssentials {
    pub kind: TransactionKind,
    pub input: Bytes,
    pub nonce: U256,
    pub gas_limit: U256,
    pub gas_price: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub value: U256,
    pub chain_id: Option<u64>,
    pub access_list: AccessList,
}

/// Queued transaction
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingTransaction {
    /// The actual transaction
    pub transaction: MaybeImpersonatedTransaction,
    /// the recovered sender of this transaction
    sender: Address,
    /// hash of `transaction`, so it can easily be reused with encoding and hashing agan
    hash: TxHash,
}

// == impl PendingTransaction ==

impl PendingTransaction {
    /// Creates a new pending transaction and tries to verify transaction and recover sender.
    pub fn new(transaction: TypedTransaction) -> Result<Self, SignatureError> {
        let sender = transaction.recover()?;
        Ok(Self { hash: transaction.hash(), transaction: transaction.into(), sender })
    }

    /// Creates a new transaction with the given sender.
    ///
    /// In order to prevent collisions from multiple different impersonated accounts, we update the
    /// transaction's hash with the address to make it unique.
    ///
    /// See: <https://github.com/foundry-rs/foundry/issues/3759>
    #[cfg(feature = "impersonated-tx")]
    pub fn with_impersonated(transaction: TypedTransaction, sender: Address) -> Self {
        let hash = transaction.impersonated_hash(sender);
        let transaction = MaybeImpersonatedTransaction::impersonated(transaction, sender);
        Self { hash, transaction, sender }
    }

    pub fn nonce(&self) -> &U256 {
        self.transaction.nonce()
    }

    pub fn hash(&self) -> &TxHash {
        &self.hash
    }

    pub fn sender(&self) -> &Address {
        &self.sender
    }

    /// Converts the [PendingTransaction] into the [TxEnv] context that [`revm`](foundry_evm)
    /// expects.
    pub fn to_revm_tx_env(&self) -> TxEnv {
        fn transact_to(kind: &TransactionKind) -> TransactTo {
            match kind {
                TransactionKind::Call(c) => TransactTo::Call((*c).to_alloy()),
                TransactionKind::Create => TransactTo::Create(CreateScheme::Create),
            }
        }

        let caller = *self.sender();
        match &self.transaction.transaction {
            TypedTransaction::Legacy(tx) => {
                let chain_id = tx.chain_id();
                let LegacyTransaction { nonce, gas_price, gas_limit, value, kind, input, .. } = tx;
                TxEnv {
                    caller: caller.to_alloy(),
                    transact_to: transact_to(kind),
                    data: alloy_primitives::Bytes(input.0.clone()),
                    chain_id,
                    nonce: Some(nonce.as_u64()),
                    value: (*value).to_alloy(),
                    gas_price: (*gas_price).to_alloy(),
                    gas_priority_fee: None,
                    gas_limit: gas_limit.as_u64(),
                    access_list: vec![],
                }
            }
            TypedTransaction::EIP2930(tx) => {
                let EIP2930Transaction {
                    chain_id,
                    nonce,
                    gas_price,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list,
                    ..
                } = tx;
                TxEnv {
                    caller: (caller).to_alloy(),
                    transact_to: transact_to(kind),
                    data: alloy_primitives::Bytes(input.0.clone()),
                    chain_id: Some(*chain_id),
                    nonce: Some(nonce.as_u64()),
                    value: (*value).to_alloy(),
                    gas_price: (*gas_price).to_alloy(),
                    gas_priority_fee: None,
                    gas_limit: gas_limit.as_u64(),
                    access_list: to_revm_access_list(access_list.0.clone()),
                }
            }
            TypedTransaction::EIP1559(tx) => {
                let EIP1559Transaction {
                    chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list,
                    ..
                } = tx;
                TxEnv {
                    caller: (caller).to_alloy(),
                    transact_to: transact_to(kind),
                    data: alloy_primitives::Bytes(input.0.clone()),
                    chain_id: Some(*chain_id),
                    nonce: Some(nonce.as_u64()),
                    value: (*value).to_alloy(),
                    gas_price: (*max_fee_per_gas).to_alloy(),
                    gas_priority_fee: Some((*max_priority_fee_per_gas).to_alloy()),
                    gas_limit: gas_limit.as_u64(),
                    access_list: to_revm_access_list(access_list.0.clone()),
                }
            }
        }
    }
}

/// Represents all relevant information of an executed transaction
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TransactionInfo {
    pub transaction_hash: H256,
    pub transaction_index: u32,
    pub from: Address,
    pub to: Option<Address>,
    pub contract_address: Option<Address>,
    pub logs: Vec<Log>,
    pub logs_bloom: Bloom,
    pub traces: CallTraceArena,
    pub exit: InstructionResult,
    pub out: Option<Bytes>,
}

// === impl TransactionInfo ===

impl TransactionInfo {
    /// Returns the `traceAddress` of the node in the arena
    ///
    /// The `traceAddress` field of all returned traces, gives the exact location in the call trace
    /// [index in root, index in first CALL, index in second CALL, …].
    ///
    /// # Panics
    ///
    /// if the `idx` does not belong to a node
    pub fn trace_address(&self, idx: usize) -> Vec<usize> {
        if idx == 0 {
            // root call has empty traceAddress
            return vec![]
        }
        let mut graph = vec![];
        let mut node = &self.traces.arena[idx];
        while let Some(parent) = node.parent {
            // the index of the child call in the arena
            let child_idx = node.idx;
            node = &self.traces.arena[parent];
            // find the index of the child call in the parent node
            let call_idx = node
                .children
                .iter()
                .position(|child| *child == child_idx)
                .expect("child exists in parent");
            graph.push(call_idx);
        }
        graph.reverse();
        graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers_core::utils::hex;

    #[test]
    fn can_recover_sender() {
        let bytes = hex::decode("f85f800182520894095e7baea6a6c7c4c2dfeb977efac326af552d870a801ba048b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353a0efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c804").unwrap();

        let tx: TypedTransaction = rlp::decode(&bytes).expect("decoding TypedTransaction failed");
        let tx = match tx {
            TypedTransaction::Legacy(tx) => tx,
            _ => panic!("Invalid typed transaction"),
        };
        assert_eq!(tx.input, Bytes::from(b""));
        assert_eq!(tx.gas_price, U256::from(0x01u64));
        assert_eq!(tx.gas_limit, U256::from(0x5208u64));
        assert_eq!(tx.nonce, U256::from(0x00u64));
        if let TransactionKind::Call(ref to) = tx.kind {
            assert_eq!(*to, "095e7baea6a6c7c4c2dfeb977efac326af552d87".parse().unwrap());
        } else {
            panic!();
        }
        assert_eq!(tx.value, U256::from(0x0au64));
        assert_eq!(
            tx.recover().unwrap(),
            "0f65fe9276bc9a24ae7083ae28e2660ef72df99e".parse().unwrap()
        );
    }

    #[test]
    #[cfg(feature = "fastrlp")]
    fn test_decode_fastrlp_create() {
        use bytes::BytesMut;
        use open_fastrlp::Encodable;

        // tests that a contract creation tx encodes and decodes properly

        let tx = TypedTransaction::EIP2930(EIP2930Transaction {
            chain_id: 1u64,
            nonce: U256::from(0),
            gas_price: U256::from(1),
            gas_limit: U256::from(2),
            kind: TransactionKind::Create,
            value: U256::from(3),
            input: Bytes::from(vec![1, 2]),
            odd_y_parity: true,
            r: H256::default(),
            s: H256::default(),
            access_list: vec![].into(),
        });

        let mut encoded = BytesMut::new();
        tx.encode(&mut encoded);

        let decoded =
            <TypedTransaction as open_fastrlp::Decodable>::decode(&mut &*encoded).unwrap();
        assert_eq!(decoded, tx);
    }

    #[test]
    #[cfg(feature = "fastrlp")]
    fn test_decode_fastrlp_create_goerli() {
        // test that an example create tx from goerli decodes properly
        let tx_bytes =
              hex::decode("02f901ee05228459682f008459682f11830209bf8080b90195608060405234801561001057600080fd5b50610175806100206000396000f3fe608060405234801561001057600080fd5b506004361061002b5760003560e01c80630c49c36c14610030575b600080fd5b61003861004e565b604051610045919061011d565b60405180910390f35b60606020600052600f6020527f68656c6c6f2073746174656d696e64000000000000000000000000000000000060405260406000f35b600081519050919050565b600082825260208201905092915050565b60005b838110156100be5780820151818401526020810190506100a3565b838111156100cd576000848401525b50505050565b6000601f19601f8301169050919050565b60006100ef82610084565b6100f9818561008f565b93506101098185602086016100a0565b610112816100d3565b840191505092915050565b6000602082019050818103600083015261013781846100e4565b90509291505056fea264697066735822122051449585839a4ea5ac23cae4552ef8a96b64ff59d0668f76bfac3796b2bdbb3664736f6c63430008090033c080a0136ebffaa8fc8b9fda9124de9ccb0b1f64e90fbd44251b4c4ac2501e60b104f9a07eb2999eec6d185ef57e91ed099afb0a926c5b536f0155dd67e537c7476e1471")
                  .unwrap();
        let _decoded =
            <TypedTransaction as open_fastrlp::Decodable>::decode(&mut &tx_bytes[..]).unwrap();
    }

    #[test]
    #[cfg(feature = "fastrlp")]
    fn test_decode_fastrlp_call() {
        use bytes::BytesMut;
        use open_fastrlp::Encodable;

        let tx = TypedTransaction::EIP2930(EIP2930Transaction {
            chain_id: 1u64,
            nonce: U256::from(0),
            gas_price: U256::from(1),
            gas_limit: U256::from(2),
            kind: TransactionKind::Call(Address::default()),
            value: U256::from(3),
            input: Bytes::from(vec![1, 2]),
            odd_y_parity: true,
            r: H256::default(),
            s: H256::default(),
            access_list: vec![].into(),
        });

        let mut encoded = BytesMut::new();
        tx.encode(&mut encoded);

        let decoded =
            <TypedTransaction as open_fastrlp::Decodable>::decode(&mut &*encoded).unwrap();
        assert_eq!(decoded, tx);
    }

    #[test]
    #[cfg(feature = "fastrlp")]
    fn decode_transaction_consumes_buffer() {
        let bytes = &mut &hex::decode("b87502f872041a8459682f008459682f0d8252089461815774383099e24810ab832a5b2a5425c154d58829a2241af62c000080c001a059e6b67f48fb32e7e570dfb11e042b5ad2e55e3ce3ce9cd989c7e06e07feeafda0016b83f4f980694ed2eee4d10667242b1f40dc406901b34125b008d334d47469").unwrap()[..];
        let _transaction_res =
            <TypedTransaction as open_fastrlp::Decodable>::decode(bytes).unwrap();
        assert_eq!(
            bytes.len(),
            0,
            "did not consume all bytes in the buffer, {:?} remaining",
            bytes.len()
        );
    }

    #[test]
    #[cfg(feature = "fastrlp")]
    fn decode_multiple_network_txs() {
        use std::str::FromStr;

        let bytes_first = &mut &hex::decode("f86b02843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba00eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5aea03a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18").unwrap()[..];
        let expected = TypedTransaction::Legacy(LegacyTransaction {
            nonce: 2u64.into(),
            gas_price: 1000000000u64.into(),
            gas_limit: 100000u64.into(),
            kind: TransactionKind::Call(Address::from_slice(
                &hex::decode("d3e8763675e4c425df46cc3b5c0f6cbdac396046").unwrap()[..],
            )),
            value: 1000000000000000u64.into(),
            input: Bytes::default(),
            signature: Signature {
                v: 43,
                r: U256::from_str(
                    "eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5ae",
                )
                .unwrap(),
                s: U256::from_str(
                    "3a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18",
                )
                .unwrap(),
            },
        });
        assert_eq!(
            expected,
            <TypedTransaction as open_fastrlp::Decodable>::decode(bytes_first).unwrap()
        );

        let bytes_second = &mut &hex::decode("f86b01843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac3960468702769bb01b2a00802ba0e24d8bd32ad906d6f8b8d7741e08d1959df021698b19ee232feba15361587d0aa05406ad177223213df262cb66ccbb2f46bfdccfdfbbb5ffdda9e2c02d977631da").unwrap()[..];
        let expected = TypedTransaction::Legacy(LegacyTransaction {
            nonce: 1u64.into(),
            gas_price: 1000000000u64.into(),
            gas_limit: 100000u64.into(),
            kind: TransactionKind::Call(Address::from_slice(
                &hex::decode("d3e8763675e4c425df46cc3b5c0f6cbdac396046").unwrap()[..],
            )),
            value: 693361000000000u64.into(),
            input: Bytes::default(),
            signature: Signature {
                v: 43,
                r: U256::from_str(
                    "e24d8bd32ad906d6f8b8d7741e08d1959df021698b19ee232feba15361587d0a",
                )
                .unwrap(),
                s: U256::from_str(
                    "5406ad177223213df262cb66ccbb2f46bfdccfdfbbb5ffdda9e2c02d977631da",
                )
                .unwrap(),
            },
        });
        assert_eq!(
            expected,
            <TypedTransaction as open_fastrlp::Decodable>::decode(bytes_second).unwrap()
        );

        let bytes_third = &mut &hex::decode("f86b0384773594008398968094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba0ce6834447c0a4193c40382e6c57ae33b241379c5418caac9cdc18d786fd12071a03ca3ae86580e94550d7c071e3a02eadb5a77830947c9225165cf9100901bee88").unwrap()[..];
        let expected = TypedTransaction::Legacy(LegacyTransaction {
            nonce: 3u64.into(),
            gas_price: 2000000000u64.into(),
            gas_limit: 10000000u64.into(),
            kind: TransactionKind::Call(Address::from_slice(
                &hex::decode("d3e8763675e4c425df46cc3b5c0f6cbdac396046").unwrap()[..],
            )),
            value: 1000000000000000u64.into(),
            input: Bytes::default(),
            signature: Signature {
                v: 43,
                r: U256::from_str(
                    "ce6834447c0a4193c40382e6c57ae33b241379c5418caac9cdc18d786fd12071",
                )
                .unwrap(),
                s: U256::from_str(
                    "3ca3ae86580e94550d7c071e3a02eadb5a77830947c9225165cf9100901bee88",
                )
                .unwrap(),
            },
        });
        assert_eq!(
            expected,
            <TypedTransaction as open_fastrlp::Decodable>::decode(bytes_third).unwrap()
        );

        let bytes_fourth = &mut &hex::decode("b87502f872041a8459682f008459682f0d8252089461815774383099e24810ab832a5b2a5425c154d58829a2241af62c000080c001a059e6b67f48fb32e7e570dfb11e042b5ad2e55e3ce3ce9cd989c7e06e07feeafda0016b83f4f980694ed2eee4d10667242b1f40dc406901b34125b008d334d47469").unwrap()[..];
        let expected = TypedTransaction::EIP1559(EIP1559Transaction {
            chain_id: 4,
            nonce: 26u64.into(),
            max_priority_fee_per_gas: 1500000000u64.into(),
            max_fee_per_gas: 1500000013u64.into(),
            gas_limit: 21000u64.into(),
            kind: TransactionKind::Call(Address::from_slice(
                &hex::decode("61815774383099e24810ab832a5b2a5425c154d5").unwrap()[..],
            )),
            value: 3000000000000000000u64.into(),
            input: Bytes::default(),
            access_list: AccessList::default(),
            odd_y_parity: true,
            r: H256::from_str("59e6b67f48fb32e7e570dfb11e042b5ad2e55e3ce3ce9cd989c7e06e07feeafd")
                .unwrap(),
            s: H256::from_str("016b83f4f980694ed2eee4d10667242b1f40dc406901b34125b008d334d47469")
                .unwrap(),
        });
        assert_eq!(
            expected,
            <TypedTransaction as open_fastrlp::Decodable>::decode(bytes_fourth).unwrap()
        );

        let bytes_fifth = &mut &hex::decode("f8650f84832156008287fb94cf7f9e66af820a19257a2108375b180b0ec491678204d2802ca035b7bfeb9ad9ece2cbafaaf8e202e706b4cfaeb233f46198f00b44d4a566a981a0612638fb29427ca33b9a3be2a0a561beecfe0269655be160d35e72d366a6a860").unwrap()[..];
        let expected = TypedTransaction::Legacy(LegacyTransaction {
            nonce: 15u64.into(),
            gas_price: 2200000000u64.into(),
            gas_limit: 34811u64.into(),
            kind: TransactionKind::Call(Address::from_slice(
                &hex::decode("cf7f9e66af820a19257a2108375b180b0ec49167").unwrap()[..],
            )),
            value: 1234u64.into(),
            input: Bytes::default(),
            signature: Signature {
                v: 44,
                r: U256::from_str(
                    "35b7bfeb9ad9ece2cbafaaf8e202e706b4cfaeb233f46198f00b44d4a566a981",
                )
                .unwrap(),
                s: U256::from_str(
                    "612638fb29427ca33b9a3be2a0a561beecfe0269655be160d35e72d366a6a860",
                )
                .unwrap(),
            },
        });
        assert_eq!(
            expected,
            <TypedTransaction as open_fastrlp::Decodable>::decode(bytes_fifth).unwrap()
        );
    }

    // <https://github.com/gakonst/ethers-rs/issues/1732>
    #[test]
    fn test_recover_legacy_tx() {
        let raw_tx = "f9015482078b8505d21dba0083022ef1947a250d5630b4cf539739df2c5dacb4c659f2488d880c46549a521b13d8b8e47ff36ab50000000000000000000000000000000000000000000066ab5a608bd00a23f2fe000000000000000000000000000000000000000000000000000000000000008000000000000000000000000048c04ed5691981c42154c6167398f95e8f38a7ff00000000000000000000000000000000000000000000000000000000632ceac70000000000000000000000000000000000000000000000000000000000000002000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000006c6ee5e31d828de241282b9606c8e98ea48526e225a0c9077369501641a92ef7399ff81c21639ed4fd8fc69cb793cfa1dbfab342e10aa0615facb2f1bcf3274a354cfe384a38d0cc008a11c2dd23a69111bc6930ba27a8";

        let tx: TypedTransaction = rlp::decode(&hex::decode(raw_tx).unwrap()).unwrap();
        let recovered = tx.recover().unwrap();
        let expected: Address = "0xa12e1462d0ced572f396f58b6e2d03894cd7c8a4".parse().unwrap();
        assert_eq!(expected, recovered);
    }
}
