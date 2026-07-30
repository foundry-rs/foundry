use alloy_consensus::{BlobTransactionSidecarVariant, EthereumTypedTransaction};
use alloy_network::{
    BuildResult, NetworkTransactionBuilder, NetworkWallet, TransactionBuilder,
    TransactionBuilder4844, TransactionBuilderError,
};
use alloy_primitives::{Address, ChainId, TxKind, U256};
use alloy_rpc_types::{AccessList, TransactionInputKind, TransactionRequest};
#[cfg(any(test, feature = "optimism"))]
use alloy_serde::OtherFields;
use alloy_serde::WithOtherFields;
use core::num::NonZeroU64;
#[cfg(feature = "optimism")]
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, POST_EXEC_TX_TYPE_ID, TxDeposit};
#[cfg(feature = "optimism")]
use op_revm::transaction::deposit::DepositTransactionParts;
use serde::{Deserialize, Serialize};
use tempo_primitives::{
    SignatureType, TEMPO_TX_TYPE_ID, TempoTxType,
    transaction::{Call, SignedKeyAuthorization, TempoSignedAuthorization},
};

pub use tempo_alloy::rpc::TempoTransactionRequest;

#[cfg(feature = "optimism")]
use super::optimism::get_deposit_tx_parts;
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
/// - **Tempo**: When a Tempo-specific field is present, or transaction type is `TEMPO_TX_TYPE_ID`
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum FoundryTransactionRequest {
    Ethereum(TransactionRequest),
    #[cfg(feature = "optimism")]
    Op(WithOtherFields<TransactionRequest>),
    Tempo(Box<TempoTransactionRequest>),
}

const TEMPO_REQUEST_FIELDS: &[&str] = &[
    "feeToken",
    "nonceKey",
    "calls",
    "keyType",
    "keyData",
    "keyId",
    "aaAuthorizationList",
    "keyAuthorization",
    "validBefore",
    "validAfter",
    "feePayerSignature",
];

impl FoundryTransactionRequest {
    /// Returns `true` if this is an Ethereum transaction request.
    pub const fn is_ethereum(&self) -> bool {
        matches!(self, Self::Ethereum(_))
    }

    /// Returns `true` if this is an OP stack transaction request.
    #[cfg(feature = "optimism")]
    pub const fn is_op(&self) -> bool {
        matches!(self, Self::Op(_))
    }

    /// Returns `true` if this is a Tempo transaction request.
    pub const fn is_tempo(&self) -> bool {
        matches!(self, Self::Tempo(_))
    }

    /// Create a new [`FoundryTransactionRequest`] from given
    /// [`WithOtherFields<TransactionRequest>`].
    #[inline]
    pub fn new(inner: WithOtherFields<TransactionRequest>) -> serde_json::Result<Self> {
        inner.try_into()
    }

    /// Consume the [`FoundryTransactionRequest`] and return the inner transaction request.
    pub fn into_inner(self) -> TransactionRequest {
        match self {
            Self::Ethereum(tx) => tx,
            #[cfg(feature = "optimism")]
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
    #[cfg(feature = "optimism")]
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
            #[cfg(feature = "optimism")]
            Self::Op(tx) if tx.inner.transaction_type == Some(POST_EXEC_TX_TYPE_ID) => {
                FoundryTxType::PostExec
            }
            #[cfg(feature = "optimism")]
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
    #[cfg(feature = "optimism")]
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
            #[cfg(feature = "optimism")]
            FoundryTxType::Deposit => self.complete_deposit(),
            #[cfg(feature = "optimism")]
            FoundryTxType::PostExec => Err(vec!["not implemented for post-exec tx"]),
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
        #[cfg(feature = "optimism")]
        if let Ok(deposit_tx_parts) = self.get_deposit_tx_parts() {
            // Build deposit transaction
            return Ok(FoundryTypedTx::Deposit(TxDeposit {
                from: self.from().unwrap_or_default(),
                source_hash: deposit_tx_parts.source_hash,
                to: self.kind().unwrap_or_default(),
                mint: deposit_tx_parts.mint.unwrap_or_default(),
                value: self.value().unwrap_or_default(),
                gas_limit: self.gas_limit().unwrap_or_default(),
                is_system_transaction: deposit_tx_parts.is_system_transaction,
                input: self.input().cloned().unwrap_or_default(),
            }));
        }
        if self.complete_tempo().is_ok()
            && let Self::Tempo(tx_req) = self
        {
            // Build Tempo transaction
            Ok(FoundryTypedTx::Tempo(
                tx_req.build_aa().map_err(|e| Self::Tempo(Box::new(e.into_value())))?,
            ))
        } else if self.as_ref().has_eip4844_fields() && self.blob_sidecar().is_none() {
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
            #[cfg(feature = "optimism")]
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
        WithOtherFields::<TransactionRequest>::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

impl AsRef<TransactionRequest> for FoundryTransactionRequest {
    fn as_ref(&self) -> &TransactionRequest {
        match self {
            Self::Ethereum(tx) => tx,
            #[cfg(feature = "optimism")]
            Self::Op(tx) => tx,
            Self::Tempo(tx) => tx.as_ref(),
        }
    }
}

impl AsMut<TransactionRequest> for FoundryTransactionRequest {
    fn as_mut(&mut self) -> &mut TransactionRequest {
        match self {
            Self::Ethereum(tx) => tx,
            #[cfg(feature = "optimism")]
            Self::Op(tx) => tx,
            Self::Tempo(tx) => tx.as_mut(),
        }
    }
}

impl TryFrom<WithOtherFields<TransactionRequest>> for FoundryTransactionRequest {
    type Error = serde_json::Error;

    fn try_from(tx: WithOtherFields<TransactionRequest>) -> Result<Self, Self::Error> {
        #[derive(Deserialize)]
        struct NonZeroQuantity(
            #[serde(
                with = "tempo_primitives::transaction::key_authorization::serde_nonzero_quantity_opt"
            )]
            Option<NonZeroU64>,
        );

        if tx.transaction_type == Some(TEMPO_TX_TYPE_ID)
            || TEMPO_REQUEST_FIELDS.iter().any(|field| {
                tx.other.get(*field).is_some_and(|value| {
                    !value.is_null()
                        && !matches!(
                            (*field, value),
                            (
                                "calls" | "aaAuthorizationList",
                                serde_json::Value::Array(values)
                            ) if values.is_empty()
                        )
                })
            })
        {
            let mut tempo_tx_req: TempoTransactionRequest = tx.inner.into();
            tempo_tx_req.fee_token =
                tx.other.get_deserialized::<Option<Address>>("feeToken").transpose()?.flatten();
            tempo_tx_req.nonce_key =
                tx.other.get_deserialized::<Option<U256>>("nonceKey").transpose()?.flatten();
            tempo_tx_req.calls =
                tx.other.get_deserialized::<Vec<Call>>("calls").transpose()?.unwrap_or_default();
            tempo_tx_req.key_type = tx
                .other
                .get_deserialized::<Option<SignatureType>>("keyType")
                .transpose()?
                .flatten();
            tempo_tx_req.key_data =
                tx.other.get_deserialized::<Option<_>>("keyData").transpose()?.flatten();
            tempo_tx_req.key_id =
                tx.other.get_deserialized::<Option<_>>("keyId").transpose()?.flatten();
            tempo_tx_req.tempo_authorization_list = tx
                .other
                .get_deserialized::<Vec<TempoSignedAuthorization>>("aaAuthorizationList")
                .transpose()?
                .unwrap_or_default();
            tempo_tx_req.key_authorization = tx
                .other
                .get_deserialized::<Option<SignedKeyAuthorization>>("keyAuthorization")
                .transpose()?
                .flatten();
            tempo_tx_req.valid_before = tx
                .other
                .get_deserialized::<NonZeroQuantity>("validBefore")
                .transpose()?
                .and_then(|value| value.0);
            tempo_tx_req.valid_after = tx
                .other
                .get_deserialized::<NonZeroQuantity>("validAfter")
                .transpose()?
                .and_then(|value| value.0);
            tempo_tx_req.fee_payer_signature =
                tx.other.get_deserialized::<Option<_>>("feePayerSignature").transpose()?.flatten();
            return Ok(Self::Tempo(Box::new(tempo_tx_req)));
        }
        #[cfg(feature = "optimism")]
        if tx.transaction_type == Some(DEPOSIT_TX_TYPE_ID)
            || tx.transaction_type == Some(POST_EXEC_TX_TYPE_ID)
            || get_deposit_tx_parts(&tx.other).is_ok()
        {
            return Ok(Self::Op(tx));
        }
        Ok(Self::Ethereum(tx.into_inner()))
    }
}

impl From<TransactionRequest> for FoundryTransactionRequest {
    fn from(tx: TransactionRequest) -> Self {
        Self::Ethereum(tx)
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
            #[cfg(feature = "optimism")]
            FoundryTypedTx::Deposit(tx) => {
                let other = OtherFields::from_iter([
                    ("sourceHash", serde_json::to_value(tx.source_hash).unwrap()),
                    ("mint", serde_json::to_value(U256::from(tx.mint)).unwrap()),
                    ("isSystemTx", serde_json::to_value(tx.is_system_transaction).unwrap()),
                ]);
                WithOtherFields { inner: Into::<TransactionRequest>::into(tx), other }
                    .try_into()
                    .expect("valid OP transaction request")
            }
            #[cfg(feature = "optimism")]
            FoundryTypedTx::PostExec(tx) => WithOtherFields {
                inner: Into::<TransactionRequest>::into(tx),
                other: OtherFields::default(),
            }
            .try_into()
            .expect("valid OP post-exec transaction request"),
            FoundryTypedTx::Tempo(tx) => Self::Tempo(Box::new(tx.into())),
        }
    }
}

impl From<FoundryTxEnvelope> for FoundryTransactionRequest {
    fn from(tx: FoundryTxEnvelope) -> Self {
        FoundryTypedTx::from(tx).into()
    }
}

#[cfg(not(feature = "optimism"))]
impl From<alloy_rpc_types_eth::Transaction<FoundryTxEnvelope>> for FoundryTransactionRequest {
    fn from(tx: alloy_rpc_types_eth::Transaction<FoundryTxEnvelope>) -> Self {
        tx.inner.into_inner().into()
    }
}

// TransactionBuilder trait implementation for FoundryNetwork
impl TransactionBuilder for FoundryTransactionRequest {
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

    fn set_input<T: Into<alloy_primitives::Bytes>>(&mut self, input: T) {
        self.as_mut().input.input = Some(input.into());
    }

    fn set_input_kind<T: Into<alloy_primitives::Bytes>>(
        &mut self,
        input: T,
        kind: TransactionInputKind,
    ) {
        let inner = self.as_mut();
        match kind {
            TransactionInputKind::Input => inner.input.input = Some(input.into()),
            TransactionInputKind::Data => inner.input.data = Some(input.into()),
            TransactionInputKind::Both => {
                let bytes = input.into();
                inner.input.input = Some(bytes.clone());
                inner.input.data = Some(bytes);
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
}

impl NetworkTransactionBuilder<FoundryNetwork> for FoundryTransactionRequest {
    fn complete_type(&self, ty: FoundryTxType) -> Result<(), Vec<&'static str>> {
        match ty {
            FoundryTxType::Legacy => self.as_ref().complete_legacy(),
            FoundryTxType::Eip2930 => self.as_ref().complete_2930(),
            FoundryTxType::Eip1559 => self.as_ref().complete_1559(),
            FoundryTxType::Eip4844 => self.as_ref().complete_4844(),
            FoundryTxType::Eip7702 => self.as_ref().complete_7702(),
            #[cfg(feature = "optimism")]
            FoundryTxType::Deposit => self.complete_deposit(),
            #[cfg(feature = "optimism")]
            FoundryTxType::PostExec => Err(vec!["not implemented for post-exec tx"]),
            FoundryTxType::Tempo => self.complete_tempo(),
        }
    }

    fn can_submit(&self) -> bool {
        self.from().is_some()
    }

    fn can_build(&self) -> bool {
        if self.as_ref().can_build() || self.complete_tempo().is_ok() {
            return true;
        }
        #[cfg(feature = "optimism")]
        if self.complete_deposit().is_ok() {
            return true;
        }
        false
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
            #[cfg(feature = "optimism")]
            FoundryTxType::Deposit => self.complete_deposit().ok(),
            #[cfg(feature = "optimism")]
            FoundryTxType::PostExec => self.complete_type(pref).ok(),
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
        let is_deposit = {
            #[cfg(feature = "optimism")]
            {
                preferred_type == FoundryTxType::Deposit
            }
            #[cfg(not(feature = "optimism"))]
            {
                false
            }
        };
        if !is_deposit && preferred_type != FoundryTxType::Tempo {
            inner.trim_conflicting_keys();
            inner.populate_blob_hashes();
        }
        if !is_deposit {
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

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecarVariant> {
        self.as_ref().blob_sidecar()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecarVariant) {
        self.as_mut().set_blob_sidecar(sidecar);
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{B256, Bytes, Signature};
    use tempo_primitives::{
        TempoSignature, TempoTransaction,
        transaction::{Authorization, KeyAuthorization, PrimitiveSignature},
    };

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
    fn request_predicates() {
        let ethereum = FoundryTransactionRequest::default();
        assert!(ethereum.is_ethereum());
        assert!(!ethereum.is_tempo());

        let tempo = FoundryTransactionRequest::Tempo(Box::default());
        assert!(tempo.is_tempo());
        assert!(!tempo.is_ethereum());

        #[cfg(feature = "optimism")]
        {
            let op = FoundryTransactionRequest::Op(WithOtherFields::default());
            assert!(op.is_op());
            assert!(!op.is_ethereum());
            assert!(!op.is_tempo());
        }
    }

    #[test]
    fn test_routing_ethereum_default() {
        let tx = default_tx_req();
        let req: FoundryTransactionRequest = WithOtherFields::new(tx).try_into().unwrap();

        assert!(req.is_ethereum());
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Eip1559(_))));
    }

    #[test]
    fn test_routing_tempo_by_fee_token() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("feeToken".to_string(), serde_json::to_value(Address::random()).unwrap());

        let req: FoundryTransactionRequest =
            WithOtherFields { inner: tx, other }.try_into().unwrap();

        assert!(req.is_tempo());
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Tempo(_))));
    }

    #[test]
    fn test_routing_serialized_non_aa_tempo_request_to_ethereum() {
        let request = TempoTransactionRequest { inner: default_tx_req(), ..Default::default() };
        let request = serde_json::from_value::<WithOtherFields<TransactionRequest>>(
            serde_json::to_value(request).unwrap(),
        )
        .unwrap();

        let request = FoundryTransactionRequest::try_from(request).unwrap();

        assert!(matches!(request, FoundryTransactionRequest::Ethereum(_)));
        assert!(matches!(request.build_unsigned(), Ok(FoundryTypedTx::Eip1559(_))));
    }

    #[test]
    #[cfg(feature = "optimism")]
    fn test_routing_op_by_deposit_fields() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("sourceHash".to_string(), serde_json::to_value(B256::ZERO).unwrap());
        other.insert("mint".to_string(), serde_json::to_value(U256::from(1000)).unwrap());
        other.insert("isSystemTx".to_string(), serde_json::to_value(false).unwrap());

        let req: FoundryTransactionRequest =
            WithOtherFields { inner: tx, other }.try_into().unwrap();

        assert!(req.is_op());
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Deposit(_))));
    }

    #[test]
    fn test_op_incomplete_routes_to_ethereum() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        // Only provide 2 of 3 required Op fields
        other.insert("sourceHash".to_string(), serde_json::to_value(B256::ZERO).unwrap());
        other.insert("mint".to_string(), serde_json::to_value(U256::from(1000)).unwrap());

        let req: FoundryTransactionRequest =
            WithOtherFields { inner: tx, other }.try_into().unwrap();

        assert!(req.is_ethereum());
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Eip1559(_))));
    }

    #[test]
    fn test_ethereum_with_unrelated_other_fields() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("anotherField".to_string(), serde_json::to_value(123).unwrap());

        let req: FoundryTransactionRequest =
            WithOtherFields { inner: tx, other }.try_into().unwrap();

        assert!(req.is_ethereum());
        assert!(matches!(req.build_unsigned(), Ok(FoundryTypedTx::Eip1559(_))));
    }

    #[test]
    fn test_serialization_ethereum() {
        let tx = default_tx_req();
        let original: FoundryTransactionRequest = WithOtherFields::new(tx).try_into().unwrap();

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: FoundryTransactionRequest = serde_json::from_str(&serialized).unwrap();

        assert!(deserialized.is_ethereum());
    }

    #[test]
    #[cfg(feature = "optimism")]
    fn test_serialization_op() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("sourceHash".to_string(), serde_json::to_value(B256::ZERO).unwrap());
        other.insert("mint".to_string(), serde_json::to_value(U256::from(1000)).unwrap());
        other.insert("isSystemTx".to_string(), serde_json::to_value(false).unwrap());

        let original: FoundryTransactionRequest =
            WithOtherFields { inner: tx, other }.try_into().unwrap();

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: FoundryTransactionRequest = serde_json::from_str(&serialized).unwrap();

        assert!(deserialized.is_op());
    }

    #[test]
    fn test_serialization_tempo() {
        let tx = default_tx_req();
        let mut other = OtherFields::default();
        other.insert("feeToken".to_string(), serde_json::to_value(Address::ZERO).unwrap());
        other.insert("nonceKey".to_string(), serde_json::to_value(U256::from(42)).unwrap());

        let original: FoundryTransactionRequest =
            WithOtherFields { inner: tx, other }.try_into().unwrap();

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: FoundryTransactionRequest = serde_json::from_str(&serialized).unwrap();

        assert!(deserialized.is_tempo());
    }

    #[test]
    fn test_tempo_request_decodes_all_extension_fields() {
        let signature = Signature::test_signature();
        let expected = TempoTransactionRequest {
            inner: TransactionRequest {
                transaction_type: Some(TEMPO_TX_TYPE_ID),
                ..default_tx_req()
            },
            fee_token: Some(Address::repeat_byte(0x11)),
            nonce_key: Some(U256::from(42)),
            calls: vec![Call {
                to: TxKind::Call(Address::repeat_byte(0x22)),
                value: U256::from(7),
                input: Bytes::from_static(&[0xde, 0xad]),
            }],
            key_type: Some(SignatureType::WebAuthn),
            key_data: Some(Bytes::from_static(&[0xbe, 0xef])),
            key_id: Some(Address::repeat_byte(0x33)),
            tempo_authorization_list: vec![TempoSignedAuthorization::new_unchecked(
                Authorization {
                    chain_id: U256::from(4217),
                    address: Address::repeat_byte(0x44),
                    nonce: 3,
                },
                TempoSignature::default(),
            )],
            key_authorization: Some(
                KeyAuthorization::unrestricted(
                    4217,
                    SignatureType::Secp256k1,
                    Address::repeat_byte(0x55),
                )
                .into_signed(PrimitiveSignature::Secp256k1(signature)),
            ),
            valid_before: NonZeroU64::new(100),
            valid_after: NonZeroU64::new(10),
            fee_payer_signature: Some(signature),
        };
        let request = serde_json::from_value::<WithOtherFields<TransactionRequest>>(
            serde_json::to_value(&expected).unwrap(),
        )
        .unwrap();

        let decoded = FoundryTransactionRequest::try_from(request).unwrap();

        let FoundryTransactionRequest::Tempo(decoded) = decoded else { panic!() };
        assert_eq!(*decoded, expected);
    }

    #[test]
    fn test_malformed_tempo_field_is_rejected() {
        let mut other = OtherFields::default();
        other.insert("nonceKey".to_string(), serde_json::json!("not a quantity"));

        let result =
            FoundryTransactionRequest::new(WithOtherFields { inner: default_tx_req(), other });

        assert!(result.is_err());
    }

    #[test]
    fn test_tempo_validity_uses_rpc_quantities() {
        let request = serde_json::from_value::<FoundryTransactionRequest>(serde_json::json!({
            "type": "0x76",
            "validBefore": "0x64",
            "validAfter": "0xa",
        }))
        .unwrap();

        let FoundryTransactionRequest::Tempo(request) = request else { panic!() };
        assert_eq!(request.valid_before, NonZeroU64::new(100));
        assert_eq!(request.valid_after, NonZeroU64::new(10));
    }

    #[test]
    fn test_tempo_typed_request_roundtrip_preserves_fields() {
        let tx = TempoTransaction {
            chain_id: 4217,
            nonce: 7,
            fee_payer_signature: None,
            valid_before: NonZeroU64::new(100),
            valid_after: NonZeroU64::new(10),
            gas_limit: 100_000,
            max_fee_per_gas: 20,
            max_priority_fee_per_gas: 2,
            fee_token: Some(Address::random()),
            access_list: Default::default(),
            calls: vec![Call {
                to: TxKind::Call(Address::random()),
                value: U256::from(42),
                input: vec![1, 2, 3].into(),
            }],
            tempo_authorization_list: Vec::new(),
            nonce_key: U256::from(9),
            key_authorization: None,
        };

        let request: FoundryTransactionRequest = FoundryTypedTx::Tempo(tx.clone()).into();
        let rebuilt = request.build_unsigned().unwrap();

        assert_eq!(rebuilt, FoundryTypedTx::Tempo(tx));
    }

    #[test]
    #[cfg(feature = "optimism")]
    fn test_deposit_typed_tx_roundtrip() {
        let deposit_tx = TxDeposit {
            from: Address::random(),
            source_hash: B256::random(),
            to: TxKind::Call(Address::random()),
            mint: 1000u128,
            value: U256::from(500),
            gas_limit: 21000,
            is_system_transaction: true,
            input: Default::default(),
        };

        let req: FoundryTransactionRequest = FoundryTypedTx::Deposit(deposit_tx.clone()).into();

        assert!(req.is_op());

        let parts = req.get_deposit_tx_parts().expect("should parse deposit parts");
        assert_eq!(parts.source_hash, deposit_tx.source_hash);
        assert_eq!(parts.mint, Some(deposit_tx.mint));
        assert_eq!(parts.is_system_transaction, deposit_tx.is_system_transaction);
    }
}
