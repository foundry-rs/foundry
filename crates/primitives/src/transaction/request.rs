use alloy_consensus::{BlobTransactionSidecar, EthereumTypedTransaction};
use alloy_network::{
    BuildResult, DynTransactionBuilder, NetworkTransactionBuilder, NetworkWallet,
    TransactionBuilder, TransactionBuilder4844, TransactionBuilderError,
};
use alloy_primitives::{Address, B256, ChainId, TxKind, U256};
use alloy_rpc_types::{AccessList, TransactionInputKind, TransactionRequest};
use alloy_serde::{OtherFields, WithOtherFields};
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, TxDeposit};
use op_revm::transaction::deposit::DepositTransactionParts;
use serde::{Deserialize, Serialize};
use tempo_alloy::rpc::TempoTransactionRequest;
use tempo_primitives::{TEMPO_TX_TYPE_ID, TempoTxType};

use super::{FoundryTxEnvelope, FoundryTxType, FoundryTypedTx};
use crate::FoundryNetwork;

/// Foundry transaction request builder.
///
/// This is a union of different transaction request types, instantiated from a
/// [`WithOtherFields<TransactionRequest>`]. The specific variant is determined by the transaction
/// type field and/or the presence of certain fields:
/// - **Ethereum**: Default variant when no special fields are present
/// - **Op**: When `sourceHash`, `mint`, and `isSystemTx` fields are present, or transaction type is
///   `DEPOSIT_TX_TYPE_ID`
/// - **Tempo**: When `feeToken` or `nonceKey` fields are present, or transaction type is
///   `TEMPO_TX_TYPE_ID`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoundryTransactionRequest {
    Ethereum(TransactionRequest),
    Op(WithOtherFields<TransactionRequest>),
    Tempo(Box<TempoTransactionRequest>),
}

impl FoundryTransactionRequest {
    /// Create a new [`FoundryTransactionRequest`] from given
    /// [`WithOtherFields<TransactionRequest>`].
    #[inline]
    pub fn new(inner: WithOtherFields<TransactionRequest>) -> Self {
        inner.into()
    }

    /// Consume the [`FoundryTransactionRequest`] and return the inner transaction request.
    pub fn into_inner(self) -> TransactionRequest {
        match self {
            Self::Ethereum(tx) => tx,
            Self::Op(tx) => tx.inner,
            Self::Tempo(tx) => tx.inner,
        }
    }

    /// Get the deposit transaction parts from the request, calling [`get_deposit_tx_parts`] helper
    /// with OtherFields.
    ///
    /// # Returns
    /// - Ok(deposit_tx_parts) if all necessary keys are present to build a deposit transaction.
    /// - Err(missing) if some keys are missing to build a deposit transaction.
    pub fn get_deposit_tx_parts(&self) -> Result<DepositTransactionParts, Vec<&'static str>> {
        match self {
            Self::Op(tx) => get_deposit_tx_parts(&tx.other),
            // Not a deposit transaction request, so missing at least sourceHash, mint, and
            // isSystemTx
            _ => Err(vec!["sourceHash", "mint", "isSystemTx"]),
        }
    }

    /// Returns the minimal transaction type this request can be converted into based on the fields
    /// that are set. See [`TransactionRequest::preferred_type`].
    pub fn preferred_type(&self) -> FoundryTxType {
        match self {
            Self::Ethereum(tx) => tx.preferred_type().into(),
            Self::Op(_) => FoundryTxType::Deposit,
            Self::Tempo(_) => FoundryTxType::Tempo,
        }
    }

    /// Check if all necessary keys are present to build a 4844 transaction,
    /// returning a list of keys that are missing.
    ///
    /// **NOTE:** Inner [`TransactionRequest::complete_4844`] method but "sidecar" key is filtered
    /// from error.
    pub fn complete_4844(&self) -> Result<(), Vec<&'static str>> {
        match self.as_ref().complete_4844() {
            Ok(()) => Ok(()),
            Err(missing) => {
                let filtered: Vec<_> =
                    missing.into_iter().filter(|&key| key != "sidecar").collect();
                if filtered.is_empty() { Ok(()) } else { Err(filtered) }
            }
        }
    }

    /// Check if all necessary keys are present to build a Deposit transaction, returning a list of
    /// keys that are missing.
    pub fn complete_deposit(&self) -> Result<(), Vec<&'static str>> {
        self.get_deposit_tx_parts().map(|_| ())
    }

    /// Check if all necessary keys are present to build a Tempo transaction, returning a list of
    /// keys that are missing.
    pub fn complete_tempo(&self) -> Result<(), Vec<&'static str>> {
        match self {
            Self::Tempo(tx) => tx.complete_type(TempoTxType::AA).map(|_| ()),
            // Not a Tempo transaction request, so missing at least feeToken and nonceKey
            _ => Err(vec!["feeToken", "nonceKey"]),
        }
    }

    /// Check if all necessary keys are present to build a transaction.
    ///
    /// # Returns
    ///
    /// - Ok(type) if all necessary keys are present to build the preferred type.
    /// - Err((type, missing)) if some keys are missing to build the preferred type.
    pub fn missing_keys(&self) -> Result<FoundryTxType, (FoundryTxType, Vec<&'static str>)> {
        let pref = self.preferred_type();
        if let Err(missing) = match pref {
            FoundryTxType::Legacy => self.as_ref().complete_legacy(),
            FoundryTxType::Eip2930 => self.as_ref().complete_2930(),
            FoundryTxType::Eip1559 => self.as_ref().complete_1559(),
            FoundryTxType::Eip4844 => self.complete_4844(),
            FoundryTxType::Eip7702 => self.as_ref().complete_7702(),
            FoundryTxType::Deposit => self.complete_deposit(),
            FoundryTxType::Tempo => self.complete_tempo(),
        } {
            Err((pref, missing))
        } else {
            Ok(pref)
        }
    }

    /// Build a typed transaction from this request.
    ///
    /// Converts the request into a `FoundryTypedTx`, handling all Ethereum and OP-stack transaction
    /// types.
    pub fn build_typed_tx(self) -> Result<FoundryTypedTx, Self> {
        if let Ok(deposit_tx_parts) = self.get_deposit_tx_parts() {
            // Build deposit transaction
            Ok(FoundryTypedTx::Deposit(TxDeposit {
                from: self.from().unwrap_or_default(),
                source_hash: deposit_tx_parts.source_hash,
                to: self.kind().unwrap_or_default(),
                mint: deposit_tx_parts.mint.unwrap_or_default(),
                value: self.value().unwrap_or_default(),
                gas_limit: self.gas_limit().unwrap_or_default(),
                is_system_transaction: deposit_tx_parts.is_system_transaction,
                input: self.input().cloned().unwrap_or_default(),
            }))
        } else if self.complete_tempo().is_ok()
            && let Self::Tempo(tx_req) = self
        {
            // Build Tempo transaction
            Ok(FoundryTypedTx::Tempo(
                tx_req.build_aa().map_err(|e| Self::Tempo(Box::new(e.into_value())))?,
            ))
        } else if self.as_ref().has_eip4844_fields()
            && self.blob_sidecar().is_none()
            && alloy_network::TransactionBuilder7594::blob_sidecar_7594(self.as_ref()).is_none()
        {
            // if request has eip4844 fields but no blob sidecar (neither eip4844 nor eip7594
            // format), try to build to eip4844 without sidecar
            self.into_inner()
                .build_4844_without_sidecar()
                .map_err(|e| Self::Ethereum(e.into_value()))
                .map(|tx| FoundryTypedTx::Eip4844(tx.into()))
        } else {
            // Use the inner transaction request to build EthereumTypedTransaction
            let typed_tx = self.into_inner().build_typed_tx().map_err(Self::Ethereum)?;
            // Convert EthereumTypedTransaction to FoundryTypedTx
            Ok(match typed_tx {
                EthereumTypedTransaction::Legacy(tx) => FoundryTypedTx::Legacy(tx),
                EthereumTypedTransaction::Eip2930(tx) => FoundryTypedTx::Eip2930(tx),
                EthereumTypedTransaction::Eip1559(tx) => FoundryTypedTx::Eip1559(tx),
                EthereumTypedTransaction::Eip4844(tx) => FoundryTypedTx::Eip4844(tx),
                EthereumTypedTransaction::Eip7702(tx) => FoundryTypedTx::Eip7702(tx),
            })
        }
    }
}

impl Default for FoundryTransactionRequest {
    fn default() -> Self {
        Self::Ethereum(TransactionRequest::default())
    }
}

impl Serialize for FoundryTransactionRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Ethereum(tx) => tx.serialize(serializer),
            Self::Op(tx) => tx.serialize(serializer),
            Self::Tempo(tx) => tx.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for FoundryTransactionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        WithOtherFields::<TransactionRequest>::deserialize(deserializer).map(Into::<Self>::into)
    }
}

impl AsRef<TransactionRequest> for FoundryTransactionRequest {
    fn as_ref(&self) -> &TransactionRequest {
        match self {
            Self::Ethereum(tx) => tx,
            Self::Op(tx) => tx,
            Self::Tempo(tx) => tx,
        }
    }
}

impl AsMut<TransactionRequest> for FoundryTransactionRequest {
    fn as_mut(&mut self) -> &mut TransactionRequest {
        match self {
            Self::Ethereum(tx) => tx,
            Self::Op(tx) => tx,
            Self::Tempo(tx) => tx,
        }
    }
}

impl From<WithOtherFields<TransactionRequest>> for FoundryTransactionRequest {
    fn from(tx: WithOtherFields<TransactionRequest>) -> Self {
        if tx.transaction_type == Some(TEMPO_TX_TYPE_ID)
            || tx.other.contains_key("feeToken")
            || tx.other.contains_key("nonceKey")
        {
            let mut tempo_tx_req: TempoTransactionRequest = tx.inner.into();
            if let Some(fee_token) =
                tx.other.get_deserialized::<Address>("feeToken").transpose().ok().flatten()
            {
                tempo_tx_req.fee_token = Some(fee_token);
            }
            if let Some(nonce_key) =
                tx.other.get_deserialized::<U256>("nonceKey").transpose().ok().flatten()
            {
                tempo_tx_req.set_nonce_key(nonce_key);
            }
            Self::Tempo(Box::new(tempo_tx_req))
        } else if tx.transaction_type == Some(DEPOSIT_TX_TYPE_ID)
            || get_deposit_tx_parts(&tx.other).is_ok()
        {
            Self::Op(tx)
        } else {
            Self::Ethereum(tx.into_inner())
        }
    }
}

impl From<FoundryTypedTx> for FoundryTransactionRequest {
    fn from(tx: FoundryTypedTx) -> Self {
        match tx {
            FoundryTypedTx::Legacy(tx) => Self::Ethereum(Into::<TransactionRequest>::into(tx)),
            FoundryTypedTx::Eip2930(tx) => Self::Ethereum(Into::<TransactionRequest>::into(tx)),
            FoundryTypedTx::Eip1559(tx) => Self::Ethereum(Into::<TransactionRequest>::into(tx)),
            FoundryTypedTx::Eip4844(tx) => Self::Ethereum(Into::<TransactionRequest>::into(tx)),
            FoundryTypedTx::Eip7702(tx) => Self::Ethereum(Into::<TransactionRequest>::into(tx)),
            FoundryTypedTx::Deposit(tx) => {
                let other = OtherFields::from_iter([
                    ("sourceHash", tx.source_hash.to_string().into()),
                    ("mint", tx.mint.to_string().into()),
                    ("isSystemTx", tx.is_system_transaction.to_string().into()),
                ]);
                WithOtherFields { inner: Into::<TransactionRequest>::into(tx), other }.into()
            }
            FoundryTypedTx::Tempo(tx) => {
                let mut other = OtherFields::default();
                if let Some(fee_token) = tx.fee_token {
                    other.insert("feeToken".to_string(), serde_json::to_value(fee_token).unwrap());
                }
                other.insert("nonceKey".to_string(), serde_json::to_value(tx.nonce_key).unwrap());
                let first_call = tx.calls.first();
                let mut inner = TransactionRequest::default()
                    .with_chain_id(tx.chain_id)
                    .with_nonce(tx.nonce)
                    .with_gas_limit(tx.gas_limit)
                    .with_max_fee_per_gas(tx.max_fee_per_gas)
                    .with_max_priority_fee_per_gas(tx.max_priority_fee_per_gas)
                    .with_kind(first_call.map(|c| c.to).unwrap_or_default())
                    .with_value(first_call.map(|c| c.value).unwrap_or_default())
                    .with_input(first_call.map(|c| c.input.clone()).unwrap_or_default())
                    .with_access_list(tx.access_list);
                inner.transaction_type = Some(TEMPO_TX_TYPE_ID);
                WithOtherFields { inner, other }.into()
            }
        }
    }
}

impl From<FoundryTxEnvelope> for FoundryTransactionRequest {
    fn from(tx: FoundryTxEnvelope) -> Self {
        FoundryTypedTx::from(tx).into()
    }
}

impl DynTransactionBuilder for FoundryTransactionRequest {
    fn chain_id(&self) -> Option<ChainId> {
        self.as_ref().chain_id
    }

    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.as_mut().chain_id = Some(chain_id);
    }

    fn nonce(&self) -> Option<u64> {
        self.as_ref().nonce
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.as_mut().nonce = Some(nonce);
    }

    fn take_nonce(&mut self) -> Option<u64> {
        self.as_mut().nonce.take()
    }

    fn input(&self) -> Option<&alloy_primitives::Bytes> {
        self.as_ref().input.input()
    }

    fn set_input(&mut self, input: alloy_primitives::Bytes) {
        self.as_mut().input.input = Some(input);
    }

    fn set_input_kind(&mut self, input: alloy_primitives::Bytes, kind: TransactionInputKind) {
        let inner = self.as_mut();
        match kind {
            TransactionInputKind::Input => inner.input.input = Some(input),
            TransactionInputKind::Data => inner.input.data = Some(input),
            TransactionInputKind::Both => {
                inner.input.input = Some(input.clone());
                inner.input.data = Some(input);
            }
        }
    }

    fn from(&self) -> Option<Address> {
        self.as_ref().from
    }

    fn set_from(&mut self, from: Address) {
        self.as_mut().from = Some(from);
    }

    fn kind(&self) -> Option<TxKind> {
        self.as_ref().to
    }

    fn clear_kind(&mut self) {
        self.as_mut().to = None;
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.as_mut().to = Some(kind);
    }

    fn value(&self) -> Option<U256> {
        self.as_ref().value
    }

    fn set_value(&mut self, value: U256) {
        self.as_mut().value = Some(value);
    }

    fn gas_price(&self) -> Option<u128> {
        self.as_ref().gas_price
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.as_mut().gas_price = Some(gas_price);
    }

    fn max_fee_per_gas(&self) -> Option<u128> {
        self.as_ref().max_fee_per_gas
    }

    fn set_max_fee_per_gas(&mut self, max_fee_per_gas: u128) {
        self.as_mut().max_fee_per_gas = Some(max_fee_per_gas);
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.as_ref().max_priority_fee_per_gas
    }

    fn set_max_priority_fee_per_gas(&mut self, max_priority_fee_per_gas: u128) {
        self.as_mut().max_priority_fee_per_gas = Some(max_priority_fee_per_gas);
    }

    fn gas_limit(&self) -> Option<u64> {
        self.as_ref().gas
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.as_mut().gas = Some(gas_limit);
    }

    fn access_list(&self) -> Option<&AccessList> {
        self.as_ref().access_list.as_ref()
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        self.as_mut().access_list = Some(access_list);
    }

    fn can_submit(&self) -> bool {
        self.from().is_some()
    }

    fn can_build(&self) -> bool {
        self.as_ref().can_build()
            || self.complete_deposit().is_ok()
            || self.complete_tempo().is_ok()
    }
}

impl TransactionBuilder for FoundryTransactionRequest {}

impl NetworkTransactionBuilder<FoundryNetwork> for FoundryTransactionRequest {
    fn complete_type(&self, ty: FoundryTxType) -> Result<(), Vec<&'static str>> {
        match ty {
            FoundryTxType::Legacy => self.as_ref().complete_legacy(),
            FoundryTxType::Eip2930 => self.as_ref().complete_2930(),
            FoundryTxType::Eip1559 => self.as_ref().complete_1559(),
            FoundryTxType::Eip4844 => self.as_ref().complete_4844(),
            FoundryTxType::Eip7702 => self.as_ref().complete_7702(),
            FoundryTxType::Deposit => self.complete_deposit(),
            FoundryTxType::Tempo => self.complete_tempo(),
        }
    }

    fn output_tx_type(&self) -> FoundryTxType {
        self.preferred_type()
    }

    fn output_tx_type_checked(&self) -> Option<FoundryTxType> {
        let pref = self.preferred_type();
        match pref {
            FoundryTxType::Legacy => self.as_ref().complete_legacy().ok(),
            FoundryTxType::Eip2930 => self.as_ref().complete_2930().ok(),
            FoundryTxType::Eip1559 => self.as_ref().complete_1559().ok(),
            FoundryTxType::Eip4844 => self.as_ref().complete_4844().ok(),
            FoundryTxType::Eip7702 => self.as_ref().complete_7702().ok(),
            FoundryTxType::Deposit => self.complete_deposit().ok(),
            FoundryTxType::Tempo => self.complete_tempo().ok(),
        }?;
        Some(pref)
    }

    /// Prepares [`FoundryTransactionRequest`] by trimming conflicting fields, and filling with
    /// default values the mandatory fields.
    fn prep_for_submission(&mut self) {
        let preferred_type = self.preferred_type();
        let inner = self.as_mut();
        inner.transaction_type = Some(preferred_type as u8);
        inner.gas.is_none().then(|| inner.set_gas_limit(Default::default()));
        if !matches!(preferred_type, FoundryTxType::Deposit | FoundryTxType::Tempo) {
            inner.trim_conflicting_keys();
            inner.populate_blob_hashes();
        }
        if preferred_type != FoundryTxType::Deposit {
            inner.nonce.is_none().then(|| inner.set_nonce(Default::default()));
        }
        if matches!(preferred_type, FoundryTxType::Legacy | FoundryTxType::Eip2930) {
            inner.gas_price.is_none().then(|| inner.set_gas_price(Default::default()));
        }
        if preferred_type == FoundryTxType::Eip2930 {
            inner.access_list.is_none().then(|| inner.set_access_list(Default::default()));
        }
        if matches!(
            preferred_type,
            FoundryTxType::Eip1559
                | FoundryTxType::Eip4844
                | FoundryTxType::Eip7702
                | FoundryTxType::Tempo
        ) {
            inner
                .max_priority_fee_per_gas
                .is_none()
                .then(|| inner.set_max_priority_fee_per_gas(Default::default()));
            inner.max_fee_per_gas.is_none().then(|| inner.set_max_fee_per_gas(Default::default()));
        }
        if preferred_type == FoundryTxType::Eip4844 {
            inner
                .as_ref()
                .max_fee_per_blob_gas()
                .is_none()
                .then(|| inner.as_mut().set_max_fee_per_blob_gas(Default::default()));
        }
    }

    fn build_unsigned(self) -> BuildResult<FoundryTypedTx, FoundryNetwork> {
        if let Err((tx_type, missing)) = self.missing_keys() {
            return Err(TransactionBuilderError::InvalidTransactionRequest(tx_type, missing)
                .into_unbuilt(self));
        }
        Ok(self.build_typed_tx().expect("checked by missing_keys"))
    }

    async fn build<W: NetworkWallet<FoundryNetwork>>(
        self,
        wallet: &W,
    ) -> Result<FoundryTxEnvelope, TransactionBuilderError<FoundryNetwork>> {
        Ok(wallet.sign_request(self).await?)
    }
}

impl TransactionBuilder4844 for FoundryTransactionRequest {
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.as_ref().max_fee_per_blob_gas()
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.as_mut().set_max_fee_per_blob_gas(max_fee_per_blob_gas);
    }

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecar> {
        self.as_ref().blob_sidecar()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecar) {
        self.as_mut().set_blob_sidecar(sidecar);
    }
}

/// Converts `OtherFields` to `DepositTransactionParts`, produces error with missing fields
pub fn get_deposit_tx_parts(
    other: &OtherFields,
) -> Result<DepositTransactionParts, Vec<&'static str>> {
    let mut missing = Vec::new();
    let source_hash =
        other.get_deserialized::<B256>("sourceHash").transpose().ok().flatten().unwrap_or_else(
            || {
                missing.push("sourceHash");
                Default::default()
            },
        );
    let mint = other
        .get_deserialized::<U256>("mint")
        .transpose()
        .unwrap_or_else(|_| {
            missing.push("mint");
            Default::default()
        })
        .map(|value| value.to::<u128>());
    let is_system_transaction =
        other.get_deserialized::<bool>("isSystemTx").transpose().ok().flatten().unwrap_or_else(
            || {
                missing.push("isSystemTx");
                Default::default()
            },
        );
    if missing.is_empty() {
        Ok(DepositTransactionParts { source_hash, mint, is_system_transaction })
    } else {
        Err(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_tx_req() -> TransactionRequest {
        TransactionRequest::default()
            .with_to(Address::random())
            .with_nonce(1)
            .with_value(U256::from(1000000))
            .with_gas_limit(1000000)
            .with_max_fee_per_gas(1000000)
            .with_max_priority_fee_per_gas(1000000)
    }

    #[test]
    fn test_routing_ethereum_default() {
        let tx = default_tx_req();
        let req: FoundryTransactionRequest = WithOtherFields::new(tx).into();

        assert!(matches!(req, FoundryTransactionRequest::Ethereum(_)));
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Eip1559(_))));
    }

    #[test]
    fn test_routing_tempo_by_fee_token() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("feeToken".to_string(), serde_json::to_value(Address::random()).unwrap());

        let req: FoundryTransactionRequest = WithOtherFields { inner: tx, other }.into();

        assert!(matches!(req, FoundryTransactionRequest::Tempo(_)));
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Tempo(_))));
    }

    #[test]
    fn test_routing_op_by_deposit_fields() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("sourceHash".to_string(), serde_json::to_value(B256::ZERO).unwrap());
        other.insert("mint".to_string(), serde_json::to_value(U256::from(1000)).unwrap());
        other.insert("isSystemTx".to_string(), serde_json::to_value(false).unwrap());

        let req: FoundryTransactionRequest = WithOtherFields { inner: tx, other }.into();

        assert!(matches!(req, FoundryTransactionRequest::Op(_)));
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Deposit(_))));
    }

    #[test]
    fn test_op_incomplete_routes_to_ethereum() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        // Only provide 2 of 3 required Op fields
        other.insert("sourceHash".to_string(), serde_json::to_value(B256::ZERO).unwrap());
        other.insert("mint".to_string(), serde_json::to_value(U256::from(1000)).unwrap());

        let req: FoundryTransactionRequest = WithOtherFields { inner: tx, other }.into();

        assert!(matches!(req, FoundryTransactionRequest::Ethereum(_)));
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Eip1559(_))));
    }

    #[test]
    fn test_ethereum_with_unrelated_other_fields() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("anotherField".to_string(), serde_json::to_value(123).unwrap());

        let req: FoundryTransactionRequest = WithOtherFields { inner: tx, other }.into();

        assert!(matches!(req, FoundryTransactionRequest::Ethereum(_)));
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Eip1559(_))));
    }

    #[test]
    fn test_serialization_ethereum() {
        let tx = default_tx_req();
        let original: FoundryTransactionRequest = WithOtherFields::new(tx).into();

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: FoundryTransactionRequest = serde_json::from_str(&serialized).unwrap();

        assert!(matches!(deserialized, FoundryTransactionRequest::Ethereum(_)));
    }

    #[test]
    fn test_serialization_op() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("sourceHash".to_string(), serde_json::to_value(B256::ZERO).unwrap());
        other.insert("mint".to_string(), serde_json::to_value(U256::from(1000)).unwrap());
        other.insert("isSystemTx".to_string(), serde_json::to_value(false).unwrap());

        let original: FoundryTransactionRequest = WithOtherFields { inner: tx, other }.into();

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: FoundryTransactionRequest = serde_json::from_str(&serialized).unwrap();

        assert!(matches!(deserialized, FoundryTransactionRequest::Op(_)));
    }

    #[test]
    fn test_serialization_tempo() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("feeToken".to_string(), serde_json::to_value(Address::ZERO).unwrap());
        other.insert("nonceKey".to_string(), serde_json::to_value(U256::from(42)).unwrap());

        let original: FoundryTransactionRequest = WithOtherFields { inner: tx, other }.into();

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: FoundryTransactionRequest = serde_json::from_str(&serialized).unwrap();

        assert!(matches!(deserialized, FoundryTransactionRequest::Tempo(_)));
    }
}
