//! transaction related data

use crate::eth::{
    receipt::Log,
    utils::{enveloped, to_access_list},
};
use ethers_core::{
    types::{
        transaction::eip2930::{AccessList, AccessListItem},
        Address, Bloom, Bytes, Signature, SignatureError, TxHash, H256, U256,
    },
    utils::{
        keccak256, rlp,
        rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream},
    },
};
use foundry_evm::{
    revm::{CreateScheme, TransactTo, TxEnv},
    trace::node::CallTraceNode,
};
use serde::{Deserialize, Serialize};

mod ethers_compat;

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
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct EthTransactionRequest {
    /// from address
    pub from: Option<Address>,
    /// to address
    pub to: Option<Address>,
    /// legacy, gas Price
    #[serde(default)]
    pub gas_price: Option<U256>,
    /// max base fee per gas sender is willing to pay
    #[serde(default)]
    pub max_fee_per_gas: Option<U256>,
    /// miner tip
    #[serde(default)]
    pub max_priority_fee_per_gas: Option<U256>,
    /// gas
    pub gas: Option<U256>,
    /// value of th tx in wei
    pub value: Option<U256>,
    /// Any additional data sent
    pub data: Option<Bytes>,
    /// Transaction nonce
    pub nonce: Option<U256>,
    /// warm storage access pre-payment
    #[serde(default)]
    pub access_list: Option<Vec<AccessListItem>>,
    /// EIP-2718 type
    #[serde(rename = "type")]
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
            ..
        } = self;
        match (gas_price, max_fee_per_gas, access_list.take()) {
            // legacy transaction
            (Some(_), None, None) => {
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
                    chain_id: None,
                }))
            }
            // EIP2930
            (_, None, Some(access_list)) => {
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
                    chain_id: 0,
                    access_list,
                }))
            }
            // EIP1559
            (None, Some(_), access_list) | (None, None, access_list @ None) => {
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
                    chain_id: 0,
                    access_list: access_list.unwrap_or_default(),
                }))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

    pub fn hash(&self) -> H256 {
        match self {
            TypedTransaction::Legacy(t) => t.hash(),
            TypedTransaction::EIP2930(t) => t.hash(),
            TypedTransaction::EIP1559(t) => t.hash(),
        }
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
        let first = *data.get(0).ok_or(DecoderError::Custom("empty slice"))?;
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub transaction: TypedTransaction,
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
        Ok(Self::with_sender(transaction, sender))
    }

    /// Creates a new transaction with the given sender
    pub fn with_sender(transaction: TypedTransaction, sender: Address) -> Self {
        Self { hash: transaction.hash(), transaction, sender }
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
                TransactionKind::Call(c) => TransactTo::Call(*c),
                TransactionKind::Create => TransactTo::Create(CreateScheme::Create),
            }
        }

        let caller = *self.sender();
        match &self.transaction {
            TypedTransaction::Legacy(tx) => {
                let chain_id = tx.chain_id();
                let LegacyTransaction { nonce, gas_price, gas_limit, value, kind, input, .. } = tx;
                TxEnv {
                    caller,
                    transact_to: transact_to(kind),
                    data: input.0.clone(),
                    chain_id,
                    nonce: Some(nonce.as_u64()),
                    value: *value,
                    gas_price: *gas_price,
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
                    caller,
                    transact_to: transact_to(kind),
                    data: input.0.clone(),
                    chain_id: Some(*chain_id),
                    nonce: Some(nonce.as_u64()),
                    value: *value,
                    gas_price: *gas_price,
                    gas_priority_fee: None,
                    gas_limit: gas_limit.as_u64(),
                    access_list: to_access_list(access_list.0.clone()),
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
                    caller,
                    transact_to: transact_to(kind),
                    data: input.0.clone(),
                    chain_id: Some(*chain_id),
                    nonce: Some(nonce.as_u64()),
                    value: *value,
                    gas_price: *max_fee_per_gas,
                    gas_priority_fee: Some(*max_priority_fee_per_gas),
                    gas_limit: gas_limit.as_u64(),
                    access_list: to_access_list(access_list.0.clone()),
                }
            }
        }
    }
}

/// Represents all relevant information of an executed transaction
#[derive(Debug, PartialEq, Clone, Default)]
pub struct TransactionInfo {
    pub transaction_hash: H256,
    pub transaction_index: u32,
    pub from: Address,
    pub to: Option<Address>,
    pub contract_address: Option<Address>,
    pub logs: Vec<Log>,
    pub logs_bloom: Bloom,
    pub traces: Vec<CallTraceNode>,
}

// === impl TransactionInfo ===

impl TransactionInfo {
    /// Returns the callgraph of the trace
    pub fn trace_call_graph(&self, idx: usize) -> Vec<usize> {
        let mut graph = vec![];
        let mut node = &self.traces[idx];

        while let Some(parent) = node.parent {
            node = &self.traces[parent];
            graph.push(node.trace.depth);
        }
        graph.reverse();
        graph.push(self.traces[idx].trace.depth);
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
        assert_eq!(tx.input, b"".into());
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
}
