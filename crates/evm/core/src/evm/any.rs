//! Foundry's catch-all network.
//!
//! [`FoundryAnyNetwork`] mirrors [`AnyNetwork`] but uses [`FoundryAnyTxEnvelope`], a wrapper
//! around [`AnyTxEnvelope`] that additionally implements the traits required by
//! [`FoundryEvmNetwork`](super::FoundryEvmNetwork), such as RLP decoding and signer recovery.
//! These operations delegate to the Ethereum envelope; unknown transaction types error where
//! possible and otherwise fall back to a best-effort conversion.

use std::ops::{Deref, DerefMut};

use alloy_consensus::{
    BlobTransactionSidecarVariant, Signed, Transaction as TransactionTrait, crypto::RecoveryError,
    transaction::SignerRecoverable,
};
use alloy_eips::{
    Typed2718,
    eip2718::{Decodable2718, Eip2718Result, Encodable2718},
    eip7702::SignedAuthorization,
};
use alloy_evm::FromRecoveredTx;
use alloy_network::{
    AnyHeader, AnyNetwork, AnyReceiptEnvelope, AnyRpcHeader, AnyRpcTransaction,
    AnyTransactionReceipt, AnyTxEnvelope, AnyTxType, AnyTypedTransaction, BuildResult, Network,
    NetworkTransactionBuilder, NetworkWallet, TransactionBuilderError, TransactionResponse,
    UnknownTxEnvelope,
};
use alloy_primitives::{Address, B256, BlockHash, Bytes, ChainId, TxHash, TxKind, U256};
use alloy_rlp::Decodable;
use alloy_rpc_types::{AccessList, Block, Transaction as RpcTransaction, TransactionRequest};
use alloy_serde::WithOtherFields;
use foundry_common::{FoundryTransactionBuilder, fmt::UIfmt};
use revm::{context::TxEnv, context_interface::either::Either};
use serde::{Deserialize, Serialize};

/// Foundry's catch-all [`Network`], mirroring [`AnyNetwork`] with a Foundry-owned transaction
/// envelope that supports the operations required by
/// [`FoundryEvmNetwork`](super::FoundryEvmNetwork).
#[derive(Clone, Copy, Debug, Default)]
pub struct FoundryAnyNetwork {
    _private: (),
}

impl Network for FoundryAnyNetwork {
    type TxType = AnyTxType;
    type TxEnvelope = FoundryAnyTxEnvelope;
    type UnsignedTx = AnyTypedTransaction;
    type ReceiptEnvelope = AnyReceiptEnvelope;
    type Header = AnyHeader;
    type TransactionRequest = WithOtherFields<TransactionRequest>;
    type TransactionResponse = FoundryAnyRpcTransaction;
    type ReceiptResponse = AnyTransactionReceipt;
    type HeaderResponse = AnyRpcHeader;
    type BlockResponse = WithOtherFields<Block<FoundryAnyRpcTransaction, AnyRpcHeader>>;
}

/// Wrapper around [`AnyTxEnvelope`] that supports RLP decoding and signer recovery by delegating
/// to the Ethereum envelope.
///
/// Unknown transaction types cannot be RLP decoded, and signer recovery for them returns a
/// [`RecoveryError`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FoundryAnyTxEnvelope(pub AnyTxEnvelope);

impl FoundryAnyTxEnvelope {
    /// Consumes the wrapper and returns the inner [`AnyTxEnvelope`].
    pub fn into_inner(self) -> AnyTxEnvelope {
        self.0
    }
}

impl Deref for FoundryAnyTxEnvelope {
    type Target = AnyTxEnvelope;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FoundryAnyTxEnvelope {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<AnyTxEnvelope> for FoundryAnyTxEnvelope {
    fn from(value: AnyTxEnvelope) -> Self {
        Self(value)
    }
}

impl From<FoundryAnyTxEnvelope> for AnyTxEnvelope {
    fn from(value: FoundryAnyTxEnvelope) -> Self {
        value.0
    }
}

impl From<alloy_consensus::TxEnvelope> for FoundryAnyTxEnvelope {
    fn from(value: alloy_consensus::TxEnvelope) -> Self {
        Self(AnyTxEnvelope::Ethereum(value))
    }
}

impl From<Signed<AnyTypedTransaction>> for FoundryAnyTxEnvelope {
    fn from(value: Signed<AnyTypedTransaction>) -> Self {
        Self(value.into())
    }
}

impl From<FoundryAnyTxEnvelope> for AnyTypedTransaction {
    fn from(value: FoundryAnyTxEnvelope) -> Self {
        value.0.into()
    }
}

impl From<FoundryAnyTxEnvelope> for WithOtherFields<TransactionRequest> {
    fn from(value: FoundryAnyTxEnvelope) -> Self {
        value.0.into()
    }
}

impl Typed2718 for FoundryAnyTxEnvelope {
    fn ty(&self) -> u8 {
        self.0.ty()
    }
}

impl Encodable2718 for FoundryAnyTxEnvelope {
    fn encode_2718_len(&self) -> usize {
        self.0.encode_2718_len()
    }

    fn encode_2718(&self, out: &mut dyn alloy_primitives::bytes::BufMut) {
        self.0.encode_2718(out)
    }

    fn trie_hash(&self) -> B256 {
        self.0.trie_hash()
    }
}

impl Decodable2718 for FoundryAnyTxEnvelope {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Eip2718Result<Self> {
        AnyTxEnvelope::typed_decode(ty, buf).map(Self)
    }

    fn fallback_decode(buf: &mut &[u8]) -> Eip2718Result<Self> {
        AnyTxEnvelope::fallback_decode(buf).map(Self)
    }
}

impl Decodable for FoundryAnyTxEnvelope {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        alloy_consensus::TxEnvelope::decode(buf).map(Into::into)
    }
}

impl SignerRecoverable for FoundryAnyTxEnvelope {
    fn recover_signer(&self) -> Result<Address, RecoveryError> {
        match &self.0 {
            AnyTxEnvelope::Ethereum(tx) => tx.recover_signer(),
            AnyTxEnvelope::Unknown(_) => Err(RecoveryError::new()),
        }
    }

    fn recover_signer_unchecked(&self) -> Result<Address, RecoveryError> {
        match &self.0 {
            AnyTxEnvelope::Ethereum(tx) => tx.recover_signer_unchecked(),
            AnyTxEnvelope::Unknown(_) => Err(RecoveryError::new()),
        }
    }
}

impl TransactionTrait for FoundryAnyTxEnvelope {
    fn chain_id(&self) -> Option<ChainId> {
        self.0.chain_id()
    }

    fn nonce(&self) -> u64 {
        self.0.nonce()
    }

    fn gas_limit(&self) -> u64 {
        self.0.gas_limit()
    }

    fn gas_price(&self) -> Option<u128> {
        self.0.gas_price()
    }

    fn max_fee_per_gas(&self) -> u128 {
        self.0.max_fee_per_gas()
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.0.max_priority_fee_per_gas()
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.0.max_fee_per_blob_gas()
    }

    fn priority_fee_or_price(&self) -> u128 {
        self.0.priority_fee_or_price()
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        self.0.effective_gas_price(base_fee)
    }

    fn is_dynamic_fee(&self) -> bool {
        self.0.is_dynamic_fee()
    }

    fn kind(&self) -> TxKind {
        self.0.kind()
    }

    fn is_create(&self) -> bool {
        self.0.is_create()
    }

    fn value(&self) -> U256 {
        self.0.value()
    }

    fn input(&self) -> &Bytes {
        self.0.input()
    }

    fn access_list(&self) -> Option<&AccessList> {
        self.0.access_list()
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.0.blob_versioned_hashes()
    }

    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        self.0.authorization_list()
    }
}

impl UIfmt for FoundryAnyTxEnvelope {
    fn pretty(&self) -> String {
        self.0.pretty()
    }
}

/// Wrapper around the catch-all RPC transaction using [`FoundryAnyTxEnvelope`] as the envelope
/// type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FoundryAnyRpcTransaction(pub WithOtherFields<RpcTransaction<FoundryAnyTxEnvelope>>);

impl FoundryAnyRpcTransaction {
    /// Consumes the wrapper and returns the inner RPC transaction.
    pub fn into_inner(self) -> WithOtherFields<RpcTransaction<FoundryAnyTxEnvelope>> {
        self.0
    }
}

impl Deref for FoundryAnyRpcTransaction {
    type Target = WithOtherFields<RpcTransaction<FoundryAnyTxEnvelope>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FoundryAnyRpcTransaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<FoundryAnyTxEnvelope> for FoundryAnyRpcTransaction {
    fn as_ref(&self) -> &FoundryAnyTxEnvelope {
        &self.0.inner.inner
    }
}

impl From<AnyRpcTransaction> for FoundryAnyRpcTransaction {
    fn from(tx: AnyRpcTransaction) -> Self {
        let (inner, other) = tx.into_parts();
        Self(WithOtherFields { inner: inner.map(FoundryAnyTxEnvelope), other })
    }
}

impl From<FoundryAnyRpcTransaction> for AnyRpcTransaction {
    fn from(tx: FoundryAnyRpcTransaction) -> Self {
        let WithOtherFields { inner, other } = tx.0;
        Self::new(WithOtherFields { inner: inner.map(FoundryAnyTxEnvelope::into_inner), other })
    }
}

impl From<FoundryAnyRpcTransaction> for WithOtherFields<TransactionRequest> {
    fn from(tx: FoundryAnyRpcTransaction) -> Self {
        let tx: AnyRpcTransaction = tx.into();
        tx.into()
    }
}

impl TransactionTrait for FoundryAnyRpcTransaction {
    fn chain_id(&self) -> Option<ChainId> {
        self.0.chain_id()
    }

    fn nonce(&self) -> u64 {
        self.0.nonce()
    }

    fn gas_limit(&self) -> u64 {
        self.0.gas_limit()
    }

    fn gas_price(&self) -> Option<u128> {
        TransactionTrait::gas_price(&self.0)
    }

    fn max_fee_per_gas(&self) -> u128 {
        TransactionTrait::max_fee_per_gas(&self.0)
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.0.max_priority_fee_per_gas()
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.0.max_fee_per_blob_gas()
    }

    fn priority_fee_or_price(&self) -> u128 {
        self.0.priority_fee_or_price()
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        self.0.effective_gas_price(base_fee)
    }

    fn is_dynamic_fee(&self) -> bool {
        self.0.is_dynamic_fee()
    }

    fn kind(&self) -> TxKind {
        self.0.kind()
    }

    fn is_create(&self) -> bool {
        self.0.is_create()
    }

    fn value(&self) -> U256 {
        self.0.value()
    }

    fn input(&self) -> &Bytes {
        self.0.input()
    }

    fn access_list(&self) -> Option<&AccessList> {
        self.0.access_list()
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.0.blob_versioned_hashes()
    }

    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        self.0.authorization_list()
    }
}

impl Typed2718 for FoundryAnyRpcTransaction {
    fn ty(&self) -> u8 {
        self.0.ty()
    }
}

impl TransactionResponse for FoundryAnyRpcTransaction {
    fn tx_hash(&self) -> TxHash {
        self.0.tx_hash()
    }

    fn block_hash(&self) -> Option<BlockHash> {
        TransactionResponse::block_hash(&self.0)
    }

    fn block_number(&self) -> Option<u64> {
        TransactionResponse::block_number(&self.0)
    }

    fn transaction_index(&self) -> Option<u64> {
        TransactionResponse::transaction_index(&self.0)
    }

    fn from(&self) -> Address {
        TransactionResponse::from(&self.0)
    }

    fn gas_price(&self) -> Option<u128> {
        TransactionResponse::gas_price(&self.0)
    }

    fn max_fee_per_gas(&self) -> Option<u128> {
        TransactionResponse::max_fee_per_gas(&self.0)
    }

    fn transaction_type(&self) -> Option<u8> {
        TransactionResponse::transaction_type(&self.0)
    }
}

impl NetworkTransactionBuilder<FoundryAnyNetwork> for WithOtherFields<TransactionRequest> {
    fn can_submit(&self) -> bool {
        self.deref().can_submit()
    }

    fn can_build(&self) -> bool {
        self.deref().can_build()
    }

    fn complete_type(&self, ty: AnyTxType) -> Result<(), Vec<&'static str>> {
        NetworkTransactionBuilder::<AnyNetwork>::complete_type(self, ty)
    }

    fn output_tx_type(&self) -> AnyTxType {
        NetworkTransactionBuilder::<AnyNetwork>::output_tx_type(self)
    }

    fn output_tx_type_checked(&self) -> Option<AnyTxType> {
        NetworkTransactionBuilder::<AnyNetwork>::output_tx_type_checked(self)
    }

    fn prep_for_submission(&mut self) {
        NetworkTransactionBuilder::<AnyNetwork>::prep_for_submission(self)
    }

    fn build_unsigned(self) -> BuildResult<AnyTypedTransaction, FoundryAnyNetwork> {
        if let Err((tx_type, missing)) = self.missing_keys() {
            return Err(TransactionBuilderError::InvalidTransactionRequest(
                tx_type.into(),
                missing,
            )
            .into_unbuilt(self));
        }
        Ok(self.inner.build_typed_tx().expect("checked by missing_keys").into())
    }

    async fn build<W: NetworkWallet<FoundryAnyNetwork>>(
        self,
        wallet: &W,
    ) -> Result<FoundryAnyTxEnvelope, TransactionBuilderError<FoundryAnyNetwork>> {
        Ok(wallet.sign_request(self).await?)
    }
}

impl FoundryTransactionBuilder<FoundryAnyNetwork> for WithOtherFields<TransactionRequest> {
    fn reset_gas_limit(&mut self) {
        self.gas = None;
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.max_fee_per_blob_gas
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = Some(max_fee_per_blob_gas);
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.blob_versioned_hashes.as_deref()
    }

    fn set_blob_versioned_hashes(&mut self, hashes: Vec<B256>) {
        self.blob_versioned_hashes = Some(hashes);
    }

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        self.sidecar.as_ref()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecarVariant) {
        self.sidecar = Some(sidecar);
        self.populate_blob_hashes();
    }

    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.authorization_list.as_ref()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.authorization_list = Some(authorization_list);
    }
}

impl FromRecoveredTx<FoundryAnyTxEnvelope> for TxEnv {
    fn from_recovered_tx(tx: &FoundryAnyTxEnvelope, sender: Address) -> Self {
        match &tx.0 {
            AnyTxEnvelope::Ethereum(tx) => Self::from_recovered_tx(tx, sender),
            AnyTxEnvelope::Unknown(tx) => from_unknown_tx_env(tx, sender),
        }
    }
}

/// Best-effort conversion of an [`UnknownTxEnvelope`] into a [`TxEnv`], reading the standard
/// fields through the [`Transaction`](TransactionTrait) trait. Fields the unknown transaction
/// does not carry default to zero values.
fn from_unknown_tx_env(tx: &UnknownTxEnvelope, caller: Address) -> TxEnv {
    TxEnv {
        tx_type: tx.ty(),
        caller,
        gas_limit: tx.gas_limit(),
        gas_price: tx.max_fee_per_gas(),
        kind: tx.kind(),
        value: tx.value(),
        data: tx.input().clone(),
        nonce: tx.nonce(),
        chain_id: tx.chain_id(),
        gas_priority_fee: tx.max_priority_fee_per_gas(),
        access_list: tx.access_list().cloned().unwrap_or_default(),
        blob_hashes: tx.blob_versioned_hashes().map(<[_]>::to_vec).unwrap_or_default(),
        max_fee_per_blob_gas: tx.max_fee_per_blob_gas().unwrap_or_default(),
        authorization_list: tx
            .authorization_list()
            .map(|auths| auths.iter().cloned().map(Either::Left).collect())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxEnvelope, TxLegacy};
    use alloy_primitives::{Signature, address};

    #[test]
    fn rlp_decodes_ethereum_envelope_and_recovers_signer() {
        let tx = TxLegacy {
            chain_id: Some(1),
            nonce: 2,
            gas_price: 1_000,
            gas_limit: 21_000,
            to: address!("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into(),
            value: U256::from(1u64),
            input: Bytes::new(),
        };
        let envelope = TxEnvelope::Legacy(tx.into_signed(Signature::test_signature()));

        let encoded = alloy_rlp::encode(&envelope);
        let decoded = FoundryAnyTxEnvelope::decode(&mut encoded.as_slice()).unwrap();
        assert_eq!(decoded.0, AnyTxEnvelope::Ethereum(envelope.clone()));
        assert_eq!(
            decoded.recover_signer_unchecked().unwrap(),
            envelope.recover_signer_unchecked().unwrap()
        );
    }

    #[test]
    fn unknown_transaction_converts_best_effort() {
        let tx: FoundryAnyRpcTransaction = serde_json::from_value(serde_json::json!({
            "blockHash": "0xef664d656f841b5ad6a2b527b963f1eb48b97d7889d742f6cbff6950388e24cd",
            "blockNumber": "0x73a78fd",
            "from": "0x36bde71c97b33cc4729cf772ae268934f7ab70b2",
            "gas": "0xc27a8",
            "gasPrice": "0x521",
            "hash": "0x0bf1845c5d7a82ec92365d5027f7310793d53004f3c86aa80965c67bf7e7dc80",
            "input": "0x",
            "nonce": "0x74060",
            "to": "0x4200000000000000000000000000000000000007",
            "transactionIndex": "0x1",
            "type": "0x7e",
            "value": "0x0",
            "sourceHash": "0x074adb22f2e6ed9bdd31c52eefc1f050e5db56eb85056450bccd79a6649520b3",
        }))
        .unwrap();

        let envelope: &FoundryAnyTxEnvelope = tx.as_ref();
        assert_eq!(envelope.ty(), 0x7e);
        assert!(envelope.recover_signer().is_err());

        let caller = address!("0x36bde71c97b33cc4729cf772ae268934f7ab70b2");
        let tx_env = TxEnv::from_recovered_tx(envelope, caller);
        assert_eq!(tx_env.tx_type, 0x7e);
        assert_eq!(tx_env.caller, caller);
        assert_eq!(tx_env.gas_limit, 0xc27a8);
        assert_eq!(tx_env.nonce, 0x74060);
        assert_eq!(
            tx_env.kind,
            TxKind::Call(address!("0x4200000000000000000000000000000000000007"))
        );

        let req: WithOtherFields<TransactionRequest> = tx.into();
        assert_eq!(req.inner.from, Some(caller));
        assert_eq!(req.other.get("type").and_then(serde_json::Value::as_u64), Some(0x7e));
    }
}
