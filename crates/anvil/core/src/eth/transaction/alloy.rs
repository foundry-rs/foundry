use crate::eth::{utils::eip_to_revm_access_list, transaction::optimism::{DepositTransaction, DepositTransactionRequest}};
use alloy_consensus::{ReceiptWithBloom, TxEip1559, TxEip2930, TxLegacy};
use alloy_network::{Signed, Transaction, TxKind};
use alloy_primitives::{Address, Bloom, Bytes, Log, Signature, TxHash, B256, U256};
use alloy_rlp::{Decodable, Encodable};
use alloy_rpc_types::{request::TransactionRequest, AccessList, CallRequest};
use foundry_evm::traces::CallTraceNode;
use revm::{
    interpreter::InstructionResult,
    primitives::{CreateScheme, OptimismFields, TransactTo, TxEnv},
};
use std::ops::Deref;

/// The signature used to bypass signing via the `eth_sendUnsignedTransaction` cheat RPC
#[cfg(feature = "impersonated-tx")]
pub fn impersonated_signature() -> Signature {
    Signature::from_scalars_and_parity(B256::ZERO, B256::ZERO, false).unwrap()
}

pub fn transaction_request_to_typed(tx: TransactionRequest) -> Option<TypedTransactionRequest> {
    let TransactionRequest {
        from,
        to,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas,
        value,
        data,
        nonce,
        mut access_list,
        transaction_type,
        other,
        ..
    } = tx;
    let transaction_type = transaction_type.map(|id| id.to::<u64>());

    // Special case: OP-stack deposit tx
    if transaction_type == Some(126) {
        return Some(TypedTransactionRequest::Deposit(DepositTransactionRequest {
            from: from.unwrap_or_default(),
            source_hash: other.get_deserialized::<B256>("sourceHash")?.ok()?,
            kind: TxKind::Create,
            mint: other.get_deserialized::<U256>("mint")?.ok()?,
            value: value.unwrap_or_default(),
            gas_limit: gas.unwrap_or_default(),
            is_system_tx: other.get_deserialized::<bool>("isSystemTx")?.ok()?,
            input: data.unwrap_or_default(),
        }))
    }

    match (
        transaction_type,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        access_list.take(),
    ) {
        // legacy transaction
        (Some(0), _, None, None, None) | (None, Some(_), None, None, None) => {
            Some(TypedTransactionRequest::Legacy(TxLegacy {
                nonce: nonce.unwrap_or_default().to::<u64>(),
                gas_price: gas_price.unwrap_or_default().to::<u128>(),
                gas_limit: gas.unwrap_or_default().to::<u64>(),
                value: value.unwrap_or(U256::ZERO),
                input: data.unwrap_or_default(),
                to: match to {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                chain_id: None,
            }))
        }
        // EIP2930
        (Some(1), _, None, None, _) | (None, _, None, None, Some(_)) => {
            Some(TypedTransactionRequest::EIP2930(TxEip2930 {
                nonce: nonce.unwrap_or_default().to::<u64>(),
                gas_price: gas_price.unwrap_or_default().to(),
                gas_limit: gas.unwrap_or_default().to::<u64>(),
                value: value.unwrap_or(U256::ZERO),
                input: data.unwrap_or_default(),
                to: match to {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                chain_id: 0,
                access_list: to_eip_access_list(access_list.unwrap_or_default()),
            }))
        }
        // EIP1559
        (Some(2), None, _, _, _) |
        (None, None, Some(_), _, _) |
        (None, None, _, Some(_), _) |
        (None, None, None, None, None) => {
            // Empty fields fall back to the canonical transaction schema.
            Some(TypedTransactionRequest::EIP1559(TxEip1559 {
                nonce: nonce.unwrap_or_default().to::<u64>(),
                max_fee_per_gas: max_fee_per_gas.unwrap_or_default().to::<u128>(),
                max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or_default().to::<u128>(),
                gas_limit: gas.unwrap_or_default().to::<u64>(),
                value: value.unwrap_or(U256::ZERO),
                input: data.unwrap_or_default(),
                to: match to {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                chain_id: 0,
                access_list: to_eip_access_list(access_list.unwrap_or_default()),
            }))
        }
        _ => None,
    }
}

pub fn call_request_to_typed(tx: CallRequest) -> Option<TypedTransactionRequest> {
    let CallRequest {
        to,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas,
        value,
        input,
        chain_id,
        nonce,
        mut access_list,
        transaction_type,
        ..
    } = tx;
    let chain_id = chain_id.map(|id| id.to::<u64>());
    let transaction_type = transaction_type.map(|id| id.to::<u64>());

    match (
        transaction_type,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        access_list.take(),
    ) {
        // legacy transaction
        (Some(0), _, None, None, None) | (None, Some(_), None, None, None) => {
            Some(TypedTransactionRequest::Legacy(TxLegacy {
                nonce: nonce.unwrap_or_default().to::<u64>(),
                gas_price: gas_price.unwrap_or_default().to::<u128>(),
                gas_limit: gas.unwrap_or_default().to::<u64>(),
                value: value.unwrap_or(U256::ZERO),
                input: input.try_into_unique_input().unwrap_or_default().unwrap_or_default(),
                to: match to {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                chain_id,
            }))
        }
        // EIP2930
        (Some(1), _, None, None, _) | (None, _, None, None, Some(_)) => {
            Some(TypedTransactionRequest::EIP2930(TxEip2930 {
                nonce: nonce.unwrap_or_default().to::<u64>(),
                gas_price: gas_price.unwrap_or_default().to(),
                gas_limit: gas.unwrap_or_default().to::<u64>(),
                value: value.unwrap_or(U256::ZERO),
                input: input.try_into_unique_input().unwrap_or_default().unwrap_or_default(),
                to: match to {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                chain_id: chain_id.unwrap_or_default(),
                access_list: to_eip_access_list(access_list.unwrap_or_default()),
            }))
        }
        // EIP1559
        (Some(2), None, _, _, _) |
        (None, None, Some(_), _, _) |
        (None, None, _, Some(_), _) |
        (None, None, None, None, None) => {
            // Empty fields fall back to the canonical transaction schema.
            Some(TypedTransactionRequest::EIP1559(TxEip1559 {
                nonce: nonce.unwrap_or_default().to::<u64>(),
                max_fee_per_gas: max_fee_per_gas.unwrap_or_default().to::<u128>(),
                max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or_default().to::<u128>(),
                gas_limit: gas.unwrap_or_default().to::<u64>(),
                value: value.unwrap_or(U256::ZERO),
                input: input.try_into_unique_input().unwrap_or_default().unwrap_or_default(),
                to: match to {
                    Some(to) => TxKind::Call(to),
                    None => TxKind::Create,
                },
                chain_id: chain_id.unwrap_or_default(),
                access_list: to_eip_access_list(access_list.unwrap_or_default()),
            }))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypedTransactionRequest {
    Legacy(TxLegacy),
    EIP2930(TxEip2930),
    EIP1559(TxEip1559),
    Deposit(DepositTransactionRequest),
}

/// A wrapper for [TypedTransaction] that allows impersonating accounts.
///
/// This is a helper that carries the `impersonated` sender so that the right hash
/// [TypedTransaction::impersonated_hash] can be created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaybeImpersonatedTransaction {
    pub transaction: TypedTransaction,
    pub impersonated_sender: Option<Address>,
}

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
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
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
    pub fn hash(&self) -> B256 {
        if self.transaction.is_impersonated() {
            if let Some(sender) = self.impersonated_sender {
                return self.transaction.impersonated_hash(sender)
            }
        }
        self.transaction.hash()
    }
}

impl Encodable for MaybeImpersonatedTransaction {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        self.transaction.encode(out)
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
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        TypedTransaction::decode(buf).map(MaybeImpersonatedTransaction::new)
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

impl PendingTransaction {
    pub fn new(transaction: TypedTransaction) -> Result<Self, alloy_primitives::SignatureError> {
        let sender = transaction.recover()?;
        let hash = transaction.hash();
        Ok(Self { transaction: MaybeImpersonatedTransaction::new(transaction), sender, hash })
    }

    #[cfg(feature = "impersonated-tx")]
    pub fn with_impersonated(transaction: TypedTransaction, sender: Address) -> Self {
        let hash = transaction.impersonated_hash(sender);
        Self {
            transaction: MaybeImpersonatedTransaction::impersonated(transaction, sender),
            sender,
            hash,
        }
    }

    pub fn nonce(&self) -> U256 {
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
        fn transact_to(kind: &TxKind) -> TransactTo {
            match kind {
                TxKind::Call(c) => TransactTo::Call(*c),
                TxKind::Create => TransactTo::Create(CreateScheme::Create),
            }
        }

        let caller = *self.sender();
        match &self.transaction.transaction {
            TypedTransaction::Legacy(tx) => {
                let chain_id = tx.chain_id();
                let TxLegacy { nonce, gas_price, gas_limit, value, to, input, .. } = tx.tx();
                TxEnv {
                    caller,
                    transact_to: transact_to(to),
                    data: alloy_primitives::Bytes(input.0.clone()),
                    chain_id,
                    nonce: Some(*nonce),
                    value: (*value),
                    gas_price: U256::from(*gas_price),
                    gas_priority_fee: None,
                    gas_limit: *gas_limit,
                    access_list: vec![],
                    ..Default::default()
                }
            }
            TypedTransaction::EIP2930(tx) => {
                let TxEip2930 {
                    chain_id,
                    nonce,
                    gas_price,
                    gas_limit,
                    to,
                    value,
                    input,
                    access_list,
                    ..
                } = tx.tx();
                TxEnv {
                    caller,
                    transact_to: transact_to(to),
                    data: alloy_primitives::Bytes(input.0.clone()),
                    chain_id: Some(*chain_id),
                    nonce: Some(*nonce),
                    value: *value,
                    gas_price: U256::from(*gas_price),
                    gas_priority_fee: None,
                    gas_limit: *gas_limit,
                    access_list: eip_to_revm_access_list(access_list.0.clone()),
                    ..Default::default()
                }
            }
            TypedTransaction::EIP1559(tx) => {
                let TxEip1559 {
                    chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    to,
                    value,
                    input,
                    access_list,
                    ..
                } = tx.tx();
                TxEnv {
                    caller,
                    transact_to: transact_to(to),
                    data: alloy_primitives::Bytes(input.0.clone()),
                    chain_id: Some(*chain_id),
                    nonce: Some(*nonce),
                    value: *value,
                    gas_price: U256::from(*max_fee_per_gas),
                    gas_priority_fee: Some(U256::from(*max_priority_fee_per_gas)),
                    gas_limit: *gas_limit,
                    access_list: eip_to_revm_access_list(access_list.0.clone()),
                    ..Default::default()
                }
            }
            TypedTransaction::Deposit(tx) => {
                let chain_id = tx.chain_id();
                let DepositTransaction {
                    nonce,
                    source_hash,
                    gas_limit,
                    value,
                    kind,
                    mint,
                    input,
                    is_system_tx,
                    ..
                } = tx;
                TxEnv {
                    caller,
                    transact_to: transact_to(kind),
                    data: alloy_primitives::Bytes(input.0.clone()),
                    chain_id,
                    nonce: Some(nonce.to::<u64>()),
                    value: *value,
                    gas_price: U256::ZERO,
                    gas_priority_fee: None,
                    gas_limit: gas_limit.to::<u64>(),
                    access_list: vec![],
                    optimism: OptimismFields {
                        source_hash: Some(*source_hash),
                        mint: Some(mint.to::<u128>()),
                        is_system_transaction: Some(*is_system_tx),
                        enveloped_tx: None,
                    },
                    ..Default::default()
                }
            }
        }
    }
}

/// Container type for signed, typed transactions.
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
                access_list: to_alloy_access_list(t.access_list.clone()),
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
                access_list: to_alloy_access_list(t.access_list.clone()),
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

    pub fn nonce(&self) -> U256 {
        match self {
            TypedTransaction::Legacy(t) => U256::from(t.nonce),
            TypedTransaction::EIP2930(t) => U256::from(t.nonce),
            TypedTransaction::EIP1559(t) => U256::from(t.nonce),
            TypedTransaction::Deposit(t) => U256::from(t.nonce),
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
        self.signature() == impersonated_signature()
    }

    /// Returns the hash if the transaction is impersonated (using a fake signature)
    ///
    /// This appends the `address` before hashing it
    #[cfg(feature = "impersonated-tx")]
    pub fn impersonated_hash(&self, sender: Address) -> B256 {
        let mut buffer = Vec::<u8>::new();
        Encodable::encode(self, &mut buffer);
        buffer.extend_from_slice(sender.as_ref());
        B256::from_slice(alloy_primitives::utils::keccak256(&buffer).as_slice())
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
    pub fn to(&self) -> Option<Address> {
        self.kind().to()
    }

    /// Returns the Signature of the transaction
    pub fn signature(&self) -> Signature {
        match self {
            TypedTransaction::Legacy(tx) => *tx.signature(),
            TypedTransaction::EIP2930(tx) => *tx.signature(),
            TypedTransaction::EIP1559(tx) => *tx.signature(),
            TypedTransaction::Deposit(_) => {
                Signature::from_scalars_and_parity(B256::ZERO, B256::ZERO, false).unwrap()
            }
        }
    }
}

impl Encodable for TypedTransaction {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        match self {
            TypedTransaction::Legacy(tx) => tx.encode(out),
            TypedTransaction::EIP2930(tx) => tx.encode(out),
            TypedTransaction::EIP1559(tx) => tx.encode(out),
            TypedTransaction::Deposit(tx) => tx.encode(out),
        }
    }
}

impl Decodable for TypedTransaction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        use bytes::Buf;
        use std::cmp::Ordering;

        let first = *buf.first().ok_or(alloy_rlp::Error::Custom("empty slice"))?;

        // a signed transaction is either encoded as a string (non legacy) or a list (legacy).
        // We should not consume the buffer if we are decoding a legacy transaction, so let's
        // check if the first byte is between 0x80 and 0xbf.
        match first.cmp(&alloy_rlp::EMPTY_LIST_CODE) {
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
                let _header = alloy_rlp::Header::decode(buf)?;
                let tx_type = *buf.first().ok_or(alloy_rlp::Error::Custom(
                    "typed tx cannot be decoded from an empty slice",
                ))?;
                if tx_type == 0x01 {
                    buf.advance(1);
                    <Signed<TxEip2930> as Decodable>::decode(buf).map(TypedTransaction::EIP2930)
                } else if tx_type == 0x02 {
                    buf.advance(1);
                    <Signed<TxEip1559> as Decodable>::decode(buf).map(TypedTransaction::EIP1559)
                } else if tx_type == 0x7E {
                    buf.advance(1);
                    <DepositTransaction as Decodable>::decode(buf).map(TypedTransaction::Deposit)
                } else {
                    Err(alloy_rlp::Error::Custom("invalid tx type"))
                }
            }
            Ordering::Equal => {
                Err(alloy_rlp::Error::Custom("an empty list is not a valid transaction encoding"))
            }
            Ordering::Greater => {
                <Signed<TxLegacy> as Decodable>::decode(buf).map(TypedTransaction::Legacy)
            }
        }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypedReceipt {
    Legacy(ReceiptWithBloom),
    EIP2930(ReceiptWithBloom),
    EIP1559(ReceiptWithBloom),
    Deposit(ReceiptWithBloom),
}

impl TypedReceipt {
    pub fn gas_used(&self) -> U256 {
        match self {
            TypedReceipt::Legacy(r) |
            TypedReceipt::EIP1559(r) |
            TypedReceipt::EIP2930(r) |
            TypedReceipt::Deposit(r) => U256::from(r.receipt.cumulative_gas_used),
        }
    }

    pub fn logs_bloom(&self) -> &Bloom {
        match self {
            TypedReceipt::Legacy(r) |
            TypedReceipt::EIP1559(r) |
            TypedReceipt::EIP2930(r) |
            TypedReceipt::Deposit(r) => &r.bloom,
        }
    }
}

impl Encodable for TypedReceipt {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        use alloy_rlp::Header;

        match self {
            TypedReceipt::Legacy(r) => r.encode(out),
            receipt => {
                let payload_len = match receipt {
                    TypedReceipt::EIP2930(r) => r.length() + 1,
                    TypedReceipt::EIP1559(r) => r.length() + 1,
                    TypedReceipt::Deposit(r) => r.length() + 1,
                    _ => unreachable!("receipt already matched"),
                };

                match receipt {
                    TypedReceipt::EIP2930(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        1u8.encode(out);
                        r.encode(out);
                    }
                    TypedReceipt::EIP1559(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        2u8.encode(out);
                        r.encode(out);
                    }
                    TypedReceipt::Deposit(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        0x7Eu8.encode(out);
                        r.encode(out);
                    }
                    _ => unreachable!("receipt already matched"),
                }
            }
        }
    }
}

impl Decodable for TypedReceipt {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        use alloy_rlp::Header;
        use bytes::Buf;
        use std::cmp::Ordering;

        // a receipt is either encoded as a string (non legacy) or a list (legacy).
        // We should not consume the buffer if we are decoding a legacy receipt, so let's
        // check if the first byte is between 0x80 and 0xbf.
        let rlp_type = *buf
            .first()
            .ok_or(alloy_rlp::Error::Custom("cannot decode a receipt from empty bytes"))?;

        match rlp_type.cmp(&alloy_rlp::EMPTY_LIST_CODE) {
            Ordering::Less => {
                // strip out the string header
                let _header = Header::decode(buf)?;
                let receipt_type = *buf.first().ok_or(alloy_rlp::Error::Custom(
                    "typed receipt cannot be decoded from an empty slice",
                ))?;
                if receipt_type == 0x01 {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::EIP2930)
                } else if receipt_type == 0x02 {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::EIP1559)
                } else if receipt_type == 0x7E {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::Deposit)
                } else {
                    Err(alloy_rlp::Error::Custom("invalid receipt type"))
                }
            }
            Ordering::Equal => {
                Err(alloy_rlp::Error::Custom("an empty list is not a valid receipt encoding"))
            }
            Ordering::Greater => {
                <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::Legacy)
            }
        }
    }
}

/// Translates an EIP-2930 access list to an alloy-rpc-types access list.
pub fn to_alloy_access_list(
    access_list: alloy_eips::eip2930::AccessList,
) -> alloy_rpc_types::AccessList {
    alloy_rpc_types::AccessList(
        access_list
            .0
            .into_iter()
            .map(|item| alloy_rpc_types::AccessListItem {
                address: item.address,
                storage_keys: item.storage_keys,
            })
            .collect(),
    )
}

/// Translates an alloy-rpc-types access list to an EIP-2930 access list.
pub fn to_eip_access_list(
    access_list: alloy_rpc_types::AccessList,
) -> alloy_eips::eip2930::AccessList {
    alloy_eips::eip2930::AccessList(
        access_list
            .0
            .into_iter()
            .map(|item| alloy_eips::eip2930::AccessListItem {
                address: item.address,
                storage_keys: item.storage_keys,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use alloy_consensus::Receipt;
    use alloy_primitives::{b256, hex, LogData, Signature};
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_decode_call() {
        let bytes_first = &mut &hex::decode("f86b02843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba00eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5aea03a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18").unwrap()[..];
        let decoded = TypedTransaction::decode(&mut &bytes_first[..]).unwrap();
        println!("{:?}", hex::encode(decoded.signature().as_bytes()));
        let tx = TxLegacy {
            nonce: 2u64,
            gas_price: 1000000000u64.into(),
            gas_limit: 100000u64,
            to: TxKind::Call(Address::from_slice(
                &hex::decode("d3e8763675e4c425df46cc3b5c0f6cbdac396046").unwrap()[..],
            )),
            value: U256::from(1000000000000000u64),
            input: Bytes::default(),
            chain_id: None,
        };

        let signature = Signature::from_str("0eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5ae3a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca182b").unwrap();

        let tx = TypedTransaction::Legacy(Signed::new_unchecked(
            tx.clone(),
            signature,
            b256!("a517b206d2223278f860ea017d3626cacad4f52ff51030dc9a96b432f17f8d34"),
        ));

        println!("{:#?}", decoded);
        assert_eq!(tx, decoded);
    }

    #[test]
    fn test_decode_create_goerli() {
        // test that an example create tx from goerli decodes properly
        let tx_bytes =
              hex::decode("02f901ee05228459682f008459682f11830209bf8080b90195608060405234801561001057600080fd5b50610175806100206000396000f3fe608060405234801561001057600080fd5b506004361061002b5760003560e01c80630c49c36c14610030575b600080fd5b61003861004e565b604051610045919061011d565b60405180910390f35b60606020600052600f6020527f68656c6c6f2073746174656d696e64000000000000000000000000000000000060405260406000f35b600081519050919050565b600082825260208201905092915050565b60005b838110156100be5780820151818401526020810190506100a3565b838111156100cd576000848401525b50505050565b6000601f19601f8301169050919050565b60006100ef82610084565b6100f9818561008f565b93506101098185602086016100a0565b610112816100d3565b840191505092915050565b6000602082019050818103600083015261013781846100e4565b90509291505056fea264697066735822122051449585839a4ea5ac23cae4552ef8a96b64ff59d0668f76bfac3796b2bdbb3664736f6c63430008090033c080a0136ebffaa8fc8b9fda9124de9ccb0b1f64e90fbd44251b4c4ac2501e60b104f9a07eb2999eec6d185ef57e91ed099afb0a926c5b536f0155dd67e537c7476e1471")
                  .unwrap();
        let _decoded =
            <TypedTransaction as alloy_rlp::Decodable>::decode(&mut &tx_bytes[..]).unwrap();
    }

    #[test]
    fn can_recover_sender() {
        // random mainnet tx: https://etherscan.io/tx/0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f
        let bytes = hex::decode("02f872018307910d808507204d2cb1827d0094388c818ca8b9251b393131c08a736a67ccb19297880320d04823e2701c80c001a0cf024f4815304df2867a1a74e9d2707b6abda0337d2d54a4438d453f4160f190a07ac0e6b3bc9395b5b9c8b9e6d77204a236577a5b18467b9175c01de4faa208d9").unwrap();

        let Ok(TypedTransaction::EIP1559(tx)) = TypedTransaction::decode(&mut &bytes[..]) else {
            panic!("decoding TypedTransaction failed");
        };

        assert_eq!(
            tx.hash(),
            &"0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f"
                .parse::<B256>()
                .unwrap()
        );
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0x95222290DD7278Aa3Ddd389Cc1E1d165CC4BAfe5".parse::<Address>().unwrap()
        );
    }

    #[test]
    fn can_recover_sender_not_normalized() {
        let bytes = hex::decode("f85f800182520894095e7baea6a6c7c4c2dfeb977efac326af552d870a801ba048b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353a0efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c804").unwrap();

        let Ok(TypedTransaction::Legacy(tx)) = TypedTransaction::decode(&mut &bytes[..]) else {
            panic!("decoding TypedTransaction failed");
        };

        assert_eq!(tx.input, Bytes::from(b""));
        assert_eq!(tx.gas_price, 1);
        assert_eq!(tx.gas_limit, 21000);
        assert_eq!(tx.nonce, 0);
        if let TxKind::Call(to) = tx.to {
            assert_eq!(
                to,
                "0x095e7baea6a6c7c4c2dfeb977efac326af552d87".parse::<Address>().unwrap()
            );
        } else {
            panic!("expected a call transaction");
        }
        assert_eq!(tx.value, U256::from(0x0au64));
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0f65fe9276bc9a24ae7083ae28e2660ef72df99e".parse::<Address>().unwrap()
        );
    }

    #[test]
    fn encode_legacy_receipt() {
        let expected = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let mut data = vec![];
        let receipt = TypedReceipt::Legacy(ReceiptWithBloom {
            receipt: Receipt {
                success: false,
                cumulative_gas_used: 0x1u64,
                logs: vec![Log {
                    address: Address::from_str("0000000000000000000000000000000000000011").unwrap(),
                    data: LogData::new_unchecked(
                        vec![
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000dead",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000beef",
                            )
                            .unwrap(),
                        ],
                        Bytes::from_str("0100ff").unwrap(),
                    ),
                }],
            },
            bloom: [0; 256].into(),
        });

        receipt.encode(&mut data);

        // check that the rlp length equals the length of the expected rlp
        assert_eq!(receipt.length(), expected.len());
        assert_eq!(data, expected);
    }

    #[test]
    fn decode_legacy_receipt() {
        let data = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let expected = TypedReceipt::Legacy(ReceiptWithBloom {
            receipt: Receipt {
                success: false,
                cumulative_gas_used: 0x1u64,
                logs: vec![Log {
                    address: Address::from_str("0000000000000000000000000000000000000011").unwrap(),
                    data: LogData::new_unchecked(
                        vec![
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000dead",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000beef",
                            )
                            .unwrap(),
                        ],
                        Bytes::from_str("0100ff").unwrap(),
                    ),
                }],
            },
            bloom: [0; 256].into(),
        });

        let receipt = TypedReceipt::decode(&mut &data[..]).unwrap();

        assert_eq!(receipt, expected);
    }
}
