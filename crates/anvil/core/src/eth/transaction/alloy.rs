use alloy_consensus::{TxEip2930, TxEip1559, TxLegacy};
use alloy_network::{TxKind, Signed};
use alloy_primitives::{Address, B256, Bloom, Bytes, Log, U256};
use alloy_rlp::{Encodable, Decodable, Header as RlpHeader, Error as DecodeError};
use alloy_rpc_types::AccessList;
use revm::interpreter::InstructionResult;
use foundry_evm::traces::CallTraceNode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypedTransaction {
    /// Legacy transaction type
    Legacy(Signed<TxLegacy>),
    /// EIP-2930 transaction
    EIP2930(Signed<TxEip2930>),
    /// EIP-1559 transaction
    EIP1559(Signed<TxEip1559>),
    /// op-stack deposit transaction
    Deposit(DepositTransaction),
}

impl TypedTransaction {
    /// Returns true if the transaction uses dynamic fees: EIP1559
    pub fn is_dynamic_fee(&self) -> bool {
        matches!(self, TypedTransaction::EIP1559(_))
    }

    pub fn gas_price(&self) -> U256 {
        U256::from(match self {
            TypedTransaction::Legacy(tx) => tx.gas_price,
            TypedTransaction::EIP2930(tx) => tx.gas_price,
            TypedTransaction::EIP1559(tx) => tx.max_fee_per_gas,
            TypedTransaction::Deposit(_) => 0,
        })
    }

    pub fn gas_limit(&self) -> U256 {
        U256::from(match self {
            TypedTransaction::Legacy(tx) => tx.gas_limit,
            TypedTransaction::EIP2930(tx) => tx.gas_limit,
            TypedTransaction::EIP1559(tx) => tx.gas_limit,
            TypedTransaction::Deposit(tx) => tx.gas_limit.to::<u64>(),
        })
    }

    pub fn value(&self) -> U256 {
        U256::from(match self {
            TypedTransaction::Legacy(tx) => tx.value,
            TypedTransaction::EIP2930(tx) => tx.value,
            TypedTransaction::EIP1559(tx) => tx.value,
            TypedTransaction::Deposit(tx) => tx.value,
        })
    }

    pub fn data(&self) -> &Bytes {
        match self {
            TypedTransaction::Legacy(tx) => &tx.input,
            TypedTransaction::EIP2930(tx) => &tx.input,
            TypedTransaction::EIP1559(tx) => &tx.input,
            TypedTransaction::Deposit(tx) => &tx.input,
        }
    }

    /// Returns the transaction type
    pub fn r#type(&self) -> Option<u8> {
        match self {
            TypedTransaction::Legacy(_) => None,
            TypedTransaction::EIP2930(_) => Some(1),
            TypedTransaction::EIP1559(_) => Some(2),
            TypedTransaction::Deposit(_) => Some(0x7E),
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
                kind: t.tx().to,
                input: t.input.clone(),
                nonce: U256::from(t.tx().nonce),
                gas_limit: U256::from(t.tx().gas_limit),
                gas_price: Some(U256::from(t.tx().gas_price)),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                value: t.value,
                chain_id: t.tx().chain_id,
                access_list: Default::default(),
            },
            TypedTransaction::EIP2930(t) => TransactionEssentials {
                kind: t.tx().to,
                input: t.input.clone(),
                nonce: U256::from(t.tx().nonce),
                gas_limit: U256::from(t.tx().gas_limit),
                gas_price: Some(U256::from(t.tx().gas_price)),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                value: t.value,
                chain_id: Some(t.chain_id),
                access_list: t.access_list.clone(),
            },
            TypedTransaction::EIP1559(t) => TransactionEssentials {
                kind: t.to,
                input: t.input.clone(),
                nonce: U256::from(t.nonce),
                gas_limit: U256::from(t.gas_limit),
                gas_price: None,
                max_fee_per_gas: Some(U256::from(t.max_fee_per_gas)),
                max_priority_fee_per_gas: Some(U256::from(t.max_priority_fee_per_gas)),
                value: t.value,
                chain_id: Some(t.chain_id),
                access_list: t.access_list.clone(),
            },
            TypedTransaction::Deposit(t) => TransactionEssentials {
                kind: t.kind,
                input: t.input.clone(),
                nonce: t.nonce,
                gas_limit: t.gas_limit,
                gas_price: Some(U256::from(0)),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                value: t.value,
                chain_id: t.chain_id(),
                access_list: Default::default(),
            },
        }
    }

    pub fn nonce(&self) -> &U256 {
        match self {
            TypedTransaction::Legacy(t) => &U256::from(t.nonce),
            TypedTransaction::EIP2930(t) => &U256::from(t.nonce),
            TypedTransaction::EIP1559(t) => &U256::from(t.nonce),
            TypedTransaction::Deposit(t) => &U256::from(t.nonce),
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        match self {
            TypedTransaction::Legacy(t) => t.chain_id,
            TypedTransaction::EIP2930(t) => Some(t.chain_id),
            TypedTransaction::EIP1559(t) => Some(t.chain_id),
            TypedTransaction::Deposit(t) => t.chain_id(),
        }
    }

    pub fn as_legacy(&self) -> Option<&Signed<TxLegacy>> {
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
    pub fn hash(&self) -> B256 {
        match self {
            TypedTransaction::Legacy(t) => *t.hash(),
            TypedTransaction::EIP2930(t) => *t.hash(),
            TypedTransaction::EIP1559(t) => *t.hash(),
            TypedTransaction::Deposit(t) => t.hash(),
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
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
        match self {
            TypedTransaction::Legacy(tx) => tx.recover_signer(),
            TypedTransaction::EIP2930(tx) => tx.recover_signer(),
            TypedTransaction::EIP1559(tx) => tx.recover_signer(),
            TypedTransaction::Deposit(tx) => tx.recover(),
        }
    }

    /// Returns what kind of transaction this is
    pub fn kind(&self) -> &TxKind {
        match self {
            TypedTransaction::Legacy(tx) => &tx.to,
            TypedTransaction::EIP2930(tx) => &tx.to,
            TypedTransaction::EIP1559(tx) => &tx.to,
            TypedTransaction::Deposit(tx) => &tx.kind,
        }
    }

    /// Returns the callee if this transaction is a call
    pub fn to(&self) -> Option<&Address> {
        self.kind().as_call()
    }

    /// Returns the Signature of the transaction
    pub fn signature(&self) -> Signature {
        match self {
            TypedTransaction::Legacy(tx) => tx.signature(),
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
            TypedTransaction::Deposit(_) => Signature { r: U256::zero(), s: U256::zero(), v: 0 },
        }
    }
}


#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DepositTransaction {
    pub nonce: U256,
    pub source_hash: B256,
    pub from: Address,
    pub kind: TxKind,
    pub mint: U256,
    pub value: U256,
    pub gas_limit: U256,
    pub is_system_tx: bool,
    pub input: Bytes,
}

impl DepositTransaction {
    pub fn nonce(&self) -> &U256 {
        &self.nonce
    }

    pub fn hash(&self) -> B256 {
        B256::from_slice(alloy_primitives::keccak256(&alloy_rlp::encode(self)).as_slice())
    }

    // /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
        Ok(self.from)
    }

    pub fn chain_id(&self) -> Option<u64> {
        None
    }

    /// Encodes only the transaction's fields into the desired buffer, without a RLP header.
    pub(crate) fn encode_fields(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.nonce.encode(out);
        self.source_hash.encode(out);
        self.from.encode(out);
        self.kind.encode(out);
        self.mint.encode(out);
        self.value.encode(out);
        self.gas_limit.encode(out);
        self.is_system_tx.encode(out);
        self.input.encode(out);
    }

    /// Calculates the length of the RLP-encoded transaction's fields.
    pub(crate) fn fields_len(&self) -> usize {
        let mut len = 0;
        len += self.nonce.length();
        len += self.source_hash.length();
        len += self.from.length();
        len += self.kind.length();
        len += self.mint.length();
        len += self.value.length();
        len += self.gas_limit.length();
        len += self.is_system_tx.length();
        len += self.input.length();
        len
    }

    /// Decodes the inner [TxDeposit] fields from RLP bytes.
    ///
    /// NOTE: This assumes a RLP header has already been decoded, and _just_ decodes the following
    /// RLP fields in the following order:
    ///
    /// - `nonce`
    /// - `source_hash`
    /// - `from`
    /// - `kind`
    /// - `mint`
    /// - `value`
    /// - `gas_limit`
    /// - `is_system_tx`
    /// - `input`
    pub fn decode_inner(buf: &mut &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            nonce: Decodable::decode(buf)?,
            source_hash: Decodable::decode(buf)?,
            from: Decodable::decode(buf)?,
            kind: Decodable::decode(buf)?,
            mint: Decodable::decode(buf)?,
            value: Decodable::decode(buf)?,
            gas_limit: Decodable::decode(buf)?,
            is_system_tx: Decodable::decode(buf)?,
            input: Decodable::decode(buf)?,
        })
    }
}

impl Encodable for DepositTransaction {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        RlpHeader { list: true, payload_length: self.fields_len() }.encode(out);
        self.encode_fields(out);        
    }
}

impl Decodable for DepositTransaction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let header = RlpHeader::decode(buf)?;
        let remaining_len = buf.len();

        if header.payload_length > remaining_len {
            return Err(alloy_rlp::Error::InputTooShort);
        }

        Self::decode_inner(buf)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionEssentials {
    pub kind: TxKind,
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

/// Represents all relevant information of an executed transaction
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionInfo {
    pub transaction_hash: B256,
    pub transaction_index: u32,
    pub from: Address,
    pub to: Option<Address>,
    pub contract_address: Option<Address>,
    pub logs: Vec<Log>,
    pub logs_bloom: Bloom,
    pub traces: Vec<CallTraceNode>,
    pub exit: InstructionResult,
    pub out: Option<Bytes>,
    pub nonce: u64,
}