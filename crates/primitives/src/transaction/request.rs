use alloy_consensus::EthereumTypedTransaction;
use alloy_network::{
    BuildResult, NetworkWallet, TransactionBuilder, TransactionBuilder4844, TransactionBuilderError,
};
use alloy_primitives::{Address, B256, Bytes, ChainId, TxKind, U256};
use alloy_rpc_types::{AccessList, TransactionInputKind, TransactionRequest};
use alloy_serde::{OtherFields, WithOtherFields};
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, TxDeposit};
use op_revm::transaction::deposit::DepositTransactionParts;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tempo_primitives::{
    SignatureType, TEMPO_TX_TYPE_ID, TempoTransaction,
    transaction::{Call, TempoSignedAuthorization},
};

use super::{FoundryTxEnvelope, FoundryTxType, FoundryTypedTx};
use crate::FoundryNetwork;

/// Foundry transaction request builder.
///
/// This is implemented as a wrapper around [`WithOtherFields<TransactionRequest>`],
/// which provides handling of deposit and Tempo transactions with first-class field support.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoundryTransactionRequest {
    /// Inner transaction request with other fields
    pub inner: WithOtherFields<TransactionRequest>,
    /// Optional Tempo fee token
    pub fee_token: Option<Address>,
    /// Optional 2D nonce key for Tempo transactions
    pub nonce_key: Option<U256>,
    /// Optional calls array for Tempo transactions
    pub calls: Vec<Call>,
    /// Optional key type for gas estimation of Tempo transactions
    pub key_type: Option<SignatureType>,
    /// Optional key-specific data for gas estimation (e.g., webauthn authenticator data)
    pub key_data: Option<Bytes>,
    /// Optional authorization list for Tempo transactions
    pub tempo_authorization_list: Vec<TempoSignedAuthorization>,
}

impl Serialize for FoundryTransactionRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;

        // First serialize the inner to get its fields
        let inner_value = serde_json::to_value(&self.inner).map_err(serde::ser::Error::custom)?;
        let inner_map = inner_value
            .as_object()
            .ok_or_else(|| serde::ser::Error::custom("expected inner to serialize as object"))?;

        // Count fields: inner fields + tempo fields
        let mut field_count = inner_map.len();
        if self.fee_token.is_some() {
            field_count += 1;
        }
        if self.nonce_key.is_some() {
            field_count += 1;
        }
        if !self.calls.is_empty() {
            field_count += 1;
        }
        if self.key_type.is_some() {
            field_count += 1;
        }
        if self.key_data.is_some() {
            field_count += 1;
        }
        if !self.tempo_authorization_list.is_empty() {
            field_count += 1;
        }

        // Tempo keys that should be skipped from inner.other to avoid duplicate serialization
        const TEMPO_KEYS: &[&str] =
            &["feeToken", "nonceKey", "calls", "keyType", "keyData", "aaAuthorizationList"];

        let mut map = serializer.serialize_map(Some(field_count))?;

        // Serialize inner fields (flattened), skipping Tempo keys that will be emitted separately
        for (key, value) in inner_map {
            if TEMPO_KEYS.contains(&key.as_str()) {
                continue;
            }
            map.serialize_entry(key, value)?;
        }

        // Serialize Tempo-specific fields from first-class fields
        if let Some(ref fee_token) = self.fee_token {
            map.serialize_entry("feeToken", fee_token)?;
        }
        if let Some(ref nonce_key) = self.nonce_key {
            map.serialize_entry("nonceKey", nonce_key)?;
        }
        if !self.calls.is_empty() {
            map.serialize_entry("calls", &self.calls)?;
        }
        if let Some(ref key_type) = self.key_type {
            map.serialize_entry("keyType", key_type)?;
        }
        if let Some(ref key_data) = self.key_data {
            map.serialize_entry("keyData", key_data)?;
        }
        if !self.tempo_authorization_list.is_empty() {
            map.serialize_entry("aaAuthorizationList", &self.tempo_authorization_list)?;
        }

        map.end()
    }
}

impl<'de> Deserialize<'de> for FoundryTransactionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize as a generic JSON value first
        let value = serde_json::Value::deserialize(deserializer)?;

        // Extract Tempo-specific fields before deserializing inner
        let fee_token = value.get("feeToken").and_then(|v| serde_json::from_value(v.clone()).ok());
        let nonce_key = value.get("nonceKey").and_then(|v| serde_json::from_value(v.clone()).ok());
        let calls = value
            .get("calls")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let key_type = value.get("keyType").and_then(|v| serde_json::from_value(v.clone()).ok());
        let key_data = value.get("keyData").and_then(|v| serde_json::from_value(v.clone()).ok());
        let tempo_authorization_list = value
            .get("aaAuthorizationList")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Deserialize the inner WithOtherFields<TransactionRequest>
        let inner = serde_json::from_value(value).map_err(serde::de::Error::custom)?;

        Ok(Self {
            inner,
            fee_token,
            nonce_key,
            calls,
            key_type,
            key_data,
            tempo_authorization_list,
        })
    }
}

impl FoundryTransactionRequest {
    /// Create a new [`FoundryTransactionRequest`] from given
    /// [`WithOtherFields<TransactionRequest>`].
    #[inline]
    pub fn new(inner: WithOtherFields<TransactionRequest>) -> Self {
        Self {
            inner,
            fee_token: None,
            nonce_key: None,
            calls: Vec::new(),
            key_type: None,
            key_data: None,
            tempo_authorization_list: Vec::new(),
        }
    }

    /// Consume self and return the inner [`WithOtherFields<TransactionRequest>`].
    #[inline]
    pub fn into_inner(self) -> WithOtherFields<TransactionRequest> {
        self.inner
    }

    /// Check if this is a deposit transaction.
    #[inline]
    pub fn is_deposit(&self) -> bool {
        self.inner.transaction_type == Some(DEPOSIT_TX_TYPE_ID)
    }

    /// Check if this is a Tempo transaction.
    ///
    /// Returns true if the transaction type is explicitly set to Tempo (0x76), or if
    /// any Tempo-specific fields are set (fee_token, nonce_key, calls, key_type, key_data,
    /// or tempo_authorization_list). Also checks `other` fields for compatibility with
    /// transactions built via `WithOtherFields<TransactionRequest>`.
    #[inline]
    pub fn is_tempo(&self) -> bool {
        self.inner.transaction_type == Some(TEMPO_TX_TYPE_ID)
            || self.fee_token.is_some()
            || self.nonce_key.is_some()
            || !self.calls.is_empty()
            || self.key_type.is_some()
            || self.key_data.is_some()
            || !self.tempo_authorization_list.is_empty()
            || self.inner.other.contains_key("feeToken")
            || self.inner.other.contains_key("nonceKey")
            || self.inner.other.contains_key("calls")
    }

    /// Get the Tempo fee token.
    ///
    /// Returns the first-class fee_token field, with fallback to OtherFields for compatibility.
    pub fn get_tempo_fee_token(&self) -> Option<Address> {
        self.fee_token.or_else(|| {
            self.inner.other.get_deserialized::<Address>("feeToken").transpose().ok().flatten()
        })
    }

    /// Get the Tempo nonce sequence key.
    ///
    /// Returns the first-class nonce_key field, with fallback to OtherFields for compatibility.
    pub fn get_tempo_nonce_key(&self) -> U256 {
        self.nonce_key.unwrap_or_else(|| {
            self.inner
                .other
                .get_deserialized::<U256>("nonceKey")
                .transpose()
                .ok()
                .flatten()
                .unwrap_or_default()
        })
    }

    /// Get the Tempo calls.
    ///
    /// Returns the first-class calls field, with fallback to OtherFields for compatibility.
    pub fn get_tempo_calls(&self) -> Vec<Call> {
        if !self.calls.is_empty() {
            self.calls.clone()
        } else {
            self.inner
                .other
                .get_deserialized::<Vec<Call>>("calls")
                .transpose()
                .ok()
                .flatten()
                .unwrap_or_default()
        }
    }

    /// Builder-pattern method for setting the fee token.
    pub fn with_fee_token(mut self, fee_token: Address) -> Self {
        self.fee_token = Some(fee_token);
        self
    }

    /// Builder-pattern method for setting the nonce key.
    pub fn with_nonce_key(mut self, nonce_key: U256) -> Self {
        self.nonce_key = Some(nonce_key);
        self
    }

    /// Builder-pattern method for setting the calls.
    pub fn with_calls(mut self, calls: Vec<Call>) -> Self {
        self.calls = calls;
        self
    }

    /// Builder-pattern method for setting the key type.
    pub fn with_key_type(mut self, key_type: SignatureType) -> Self {
        self.key_type = Some(key_type);
        self
    }

    /// Builder-pattern method for setting the key data.
    pub fn with_key_data(mut self, key_data: Bytes) -> Self {
        self.key_data = Some(key_data);
        self
    }

    /// Builder-pattern method for setting the tempo authorization list.
    pub fn with_tempo_authorization_list(
        mut self,
        tempo_authorization_list: Vec<TempoSignedAuthorization>,
    ) -> Self {
        self.tempo_authorization_list = tempo_authorization_list;
        self
    }

    /// Check if all necessary keys are present to build a Tempo transaction, returning a list of
    /// keys that are missing.
    pub fn complete_tempo(&self) -> Result<(), Vec<&'static str>> {
        let mut missing = Vec::new();
        if self.chain_id().is_none() {
            missing.push("chain_id");
        }
        if self.gas_limit().is_none() {
            missing.push("gas_limit");
        }
        if self.max_fee_per_gas().is_none() {
            missing.push("max_fee_per_gas");
        }
        if self.max_priority_fee_per_gas().is_none() {
            missing.push("max_priority_fee_per_gas");
        }
        if self.nonce().is_none() {
            missing.push("nonce");
        }
        if missing.is_empty() { Ok(()) } else { Err(missing) }
    }

    /// Returns the minimal transaction type this request can be converted into based on the fields
    /// that are set. See [`TransactionRequest::preferred_type`].
    pub fn preferred_type(&self) -> FoundryTxType {
        if self.is_deposit() {
            FoundryTxType::Deposit
        } else if self.is_tempo() {
            FoundryTxType::Tempo
        } else {
            self.inner.preferred_type().into()
        }
    }

    /// Check if all necessary keys are present to build a 4844 transaction,
    /// returning a list of keys that are missing.
    ///
    /// **NOTE:** Inner [`TransactionRequest::complete_4844`] method but "sidecar" key is filtered
    /// from error.
    pub fn complete_4844(&self) -> Result<(), Vec<&'static str>> {
        match self.inner.complete_4844() {
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
        get_deposit_tx_parts(&self.inner.other).map(|_| ())
    }

    /// Return the tx type this request can be built as. Computed by checking
    /// the preferred type, and then checking for completeness.
    pub fn buildable_type(&self) -> Option<FoundryTxType> {
        let pref = self.preferred_type();
        match pref {
            FoundryTxType::Legacy => self.inner.complete_legacy().ok(),
            FoundryTxType::Eip2930 => self.inner.complete_2930().ok(),
            FoundryTxType::Eip1559 => self.inner.complete_1559().ok(),
            FoundryTxType::Eip4844 => self.inner.complete_4844().ok(),
            FoundryTxType::Eip7702 => self.inner.complete_7702().ok(),
            FoundryTxType::Deposit => self.complete_deposit().ok(),
            FoundryTxType::Tempo => self.complete_tempo().ok(),
        }?;
        Some(pref)
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
            FoundryTxType::Legacy => self.inner.complete_legacy(),
            FoundryTxType::Eip2930 => self.inner.complete_2930(),
            FoundryTxType::Eip1559 => self.inner.complete_1559(),
            FoundryTxType::Eip4844 => self.complete_4844(),
            FoundryTxType::Eip7702 => self.inner.complete_7702(),
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
        // Handle deposit transactions
        if let Ok(deposit_tx_parts) = get_deposit_tx_parts(&self.inner.other) {
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
        } else if self.is_tempo() {
            // Build Tempo transaction from first-class fields
            let mut calls = self.get_tempo_calls();
            // If no explicit calls, create one from to/value/input
            if calls.is_empty() {
                calls.push(Call {
                    to: self.kind().unwrap_or_default(),
                    value: self.value().unwrap_or_default(),
                    input: self.input().cloned().unwrap_or_default(),
                });
            }
            Ok(FoundryTypedTx::Tempo(TempoTransaction {
                chain_id: self.chain_id().unwrap_or_default(),
                fee_token: self.get_tempo_fee_token(),
                max_fee_per_gas: self.max_fee_per_gas().unwrap_or_default(),
                max_priority_fee_per_gas: self.max_priority_fee_per_gas().unwrap_or_default(),
                gas_limit: self.gas_limit().unwrap_or_default(),
                nonce_key: self.get_tempo_nonce_key(),
                nonce: self.nonce().unwrap_or_default(),
                calls,
                access_list: self.access_list().cloned().unwrap_or_default(),
                tempo_authorization_list: self.tempo_authorization_list,
                ..Default::default()
            }))
        } else if self.inner.has_eip4844_fields() && self.inner.blob_sidecar().is_none() {
            // if request has eip4844 fields but no blob sidecar, try to build to eip4844 without
            // sidecar
            self.inner
                .into_inner()
                .build_4844_without_sidecar()
                .map_err(|e| Self::new(e.into_value().into()))
                .map(|tx| FoundryTypedTx::Eip4844(tx.into()))
        } else {
            // Use the inner transaction request to build EthereumTypedTransaction
            let typed_tx =
                self.inner.into_inner().build_typed_tx().map_err(|tx| Self::new(tx.into()))?;
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

impl From<FoundryTypedTx> for FoundryTransactionRequest {
    fn from(tx: FoundryTypedTx) -> Self {
        match tx {
            FoundryTypedTx::Legacy(tx) => Self::new(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip2930(tx) => Self::new(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip1559(tx) => Self::new(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip4844(tx) => Self::new(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip7702(tx) => Self::new(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Deposit(tx) => {
                let other = OtherFields::from_iter([
                    ("sourceHash", tx.source_hash.to_string().into()),
                    ("mint", tx.mint.to_string().into()),
                    ("isSystemTx", tx.is_system_transaction.to_string().into()),
                ]);
                Self::new(WithOtherFields { inner: Into::<TransactionRequest>::into(tx), other })
            }
            FoundryTypedTx::Tempo(tx) => {
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
                Self {
                    inner: WithOtherFields { inner, other: OtherFields::default() },
                    fee_token: tx.fee_token,
                    nonce_key: Some(tx.nonce_key),
                    calls: tx.calls,
                    key_type: None,
                    key_data: None,
                    tempo_authorization_list: tx.tempo_authorization_list,
                }
            }
        }
    }
}

impl From<FoundryTxEnvelope> for FoundryTransactionRequest {
    fn from(tx: FoundryTxEnvelope) -> Self {
        FoundryTypedTx::from(tx).into()
    }
}

impl AsRef<WithOtherFields<TransactionRequest>> for FoundryTransactionRequest {
    fn as_ref(&self) -> &WithOtherFields<TransactionRequest> {
        &self.inner
    }
}

impl AsMut<WithOtherFields<TransactionRequest>> for FoundryTransactionRequest {
    fn as_mut(&mut self) -> &mut WithOtherFields<TransactionRequest> {
        &mut self.inner
    }
}

impl From<WithOtherFields<TransactionRequest>> for FoundryTransactionRequest {
    fn from(inner: WithOtherFields<TransactionRequest>) -> Self {
        Self::new(inner)
    }
}

impl From<FoundryTransactionRequest> for WithOtherFields<TransactionRequest> {
    fn from(req: FoundryTransactionRequest) -> Self {
        req.inner
    }
}

// TransactionBuilder trait implementation for FoundryNetwork
impl TransactionBuilder<FoundryNetwork> for FoundryTransactionRequest {
    fn chain_id(&self) -> Option<ChainId> {
        self.inner.chain_id
    }

    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.inner.chain_id = Some(chain_id);
    }

    fn nonce(&self) -> Option<u64> {
        self.inner.nonce
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.inner.nonce = Some(nonce);
    }

    fn take_nonce(&mut self) -> Option<u64> {
        self.inner.nonce.take()
    }

    fn input(&self) -> Option<&alloy_primitives::Bytes> {
        self.inner.input.input()
    }

    fn set_input<T: Into<alloy_primitives::Bytes>>(&mut self, input: T) {
        self.inner.input.input = Some(input.into());
    }

    fn set_input_kind<T: Into<alloy_primitives::Bytes>>(
        &mut self,
        input: T,
        kind: TransactionInputKind,
    ) {
        match kind {
            TransactionInputKind::Input => self.inner.input.input = Some(input.into()),
            TransactionInputKind::Data => self.inner.input.data = Some(input.into()),
            TransactionInputKind::Both => {
                let bytes = input.into();
                self.inner.input.input = Some(bytes.clone());
                self.inner.input.data = Some(bytes);
            }
        }
    }

    fn from(&self) -> Option<Address> {
        self.inner.from
    }

    fn set_from(&mut self, from: Address) {
        self.inner.from = Some(from);
    }

    fn kind(&self) -> Option<TxKind> {
        self.inner.to
    }

    fn clear_kind(&mut self) {
        self.inner.to = None;
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.inner.to = Some(kind);
    }

    fn value(&self) -> Option<U256> {
        self.inner.value
    }

    fn set_value(&mut self, value: U256) {
        self.inner.value = Some(value);
    }

    fn gas_price(&self) -> Option<u128> {
        self.inner.gas_price
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.inner.gas_price = Some(gas_price);
    }

    fn max_fee_per_gas(&self) -> Option<u128> {
        self.inner.max_fee_per_gas
    }

    fn set_max_fee_per_gas(&mut self, max_fee_per_gas: u128) {
        self.inner.max_fee_per_gas = Some(max_fee_per_gas);
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.inner.max_priority_fee_per_gas
    }

    fn set_max_priority_fee_per_gas(&mut self, max_priority_fee_per_gas: u128) {
        self.inner.max_priority_fee_per_gas = Some(max_priority_fee_per_gas);
    }

    fn gas_limit(&self) -> Option<u64> {
        self.inner.gas
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.inner.gas = Some(gas_limit);
    }

    fn access_list(&self) -> Option<&AccessList> {
        self.inner.access_list.as_ref()
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        self.inner.access_list = Some(access_list);
    }

    fn complete_type(&self, ty: FoundryTxType) -> Result<(), Vec<&'static str>> {
        match ty {
            FoundryTxType::Legacy => self.inner.complete_legacy(),
            FoundryTxType::Eip2930 => self.inner.complete_2930(),
            FoundryTxType::Eip1559 => self.inner.complete_1559(),
            FoundryTxType::Eip4844 => self.inner.complete_4844(),
            FoundryTxType::Eip7702 => self.inner.complete_7702(),
            FoundryTxType::Deposit => self.complete_deposit(),
            FoundryTxType::Tempo => self.complete_tempo(),
        }
    }

    fn can_submit(&self) -> bool {
        self.from().is_some()
    }

    fn can_build(&self) -> bool {
        self.inner.can_build() || get_deposit_tx_parts(&self.inner.other).is_ok() || self.is_tempo()
    }

    fn output_tx_type(&self) -> FoundryTxType {
        self.preferred_type()
    }

    fn output_tx_type_checked(&self) -> Option<FoundryTxType> {
        self.buildable_type()
    }

    fn prep_for_submission(&mut self) {
        let preferred_type = self.preferred_type();
        self.inner.transaction_type = Some(preferred_type as u8);
        self.inner.gas_limit().is_none().then(|| self.inner.set_gas_limit(Default::default()));
        if !matches!(preferred_type, FoundryTxType::Deposit | FoundryTxType::Tempo) {
            self.inner.trim_conflicting_keys();
            self.inner.populate_blob_hashes();
        }
        if preferred_type != FoundryTxType::Deposit {
            self.inner.nonce().is_none().then(|| self.inner.set_nonce(Default::default()));
        }
        if matches!(preferred_type, FoundryTxType::Legacy | FoundryTxType::Eip2930) {
            self.inner.gas_price().is_none().then(|| self.inner.set_gas_price(Default::default()));
        }
        if preferred_type == FoundryTxType::Eip2930 {
            self.inner
                .access_list()
                .is_none()
                .then(|| self.inner.set_access_list(Default::default()));
        }
        if matches!(
            preferred_type,
            FoundryTxType::Eip1559
                | FoundryTxType::Eip4844
                | FoundryTxType::Eip7702
                | FoundryTxType::Tempo
        ) {
            self.inner
                .max_priority_fee_per_gas()
                .is_none()
                .then(|| self.inner.set_max_priority_fee_per_gas(Default::default()));
            self.inner
                .max_fee_per_gas()
                .is_none()
                .then(|| self.inner.set_max_fee_per_gas(Default::default()));
        }
        if preferred_type == FoundryTxType::Eip4844 {
            self.inner
                .max_fee_per_blob_gas()
                .is_none()
                .then(|| self.inner.set_max_fee_per_blob_gas(Default::default()));
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
