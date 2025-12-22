use alloy_consensus::EthereumTypedTransaction;
use alloy_network::{
    BuildResult, NetworkWallet, TransactionBuilder, TransactionBuilder4844, TransactionBuilderError,
};
use alloy_primitives::{Address, B256, ChainId, TxKind, U256};
use alloy_rpc_types::{AccessList, TransactionInputKind, TransactionRequest};
use alloy_serde::{OtherFields, WithOtherFields};
use derive_more::{AsMut, AsRef, From, Into};
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, TxDeposit};
use op_revm::transaction::deposit::DepositTransactionParts;
use serde::{Deserialize, Serialize};

use super::{FoundryTxEnvelope, FoundryTxType, FoundryTypedTx};
use crate::FoundryNetwork;

/// Foundry transaction request builder.
///
/// This is implemented as a wrapper around [`WithOtherFields<TransactionRequest>`],
/// which provides handling of deposit transactions.
#[derive(Clone, Debug, Default, PartialEq, Eq, From, Into, AsRef, AsMut)]
pub struct FoundryTransactionRequest(WithOtherFields<TransactionRequest>);

impl Serialize for FoundryTransactionRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FoundryTransactionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        WithOtherFields::<TransactionRequest>::deserialize(deserializer).map(Self)
    }
}

impl FoundryTransactionRequest {
    /// Create a new [`FoundryTransactionRequest`] from given
    /// [`WithOtherFields<TransactionRequest>`].
    #[inline]
    pub fn new(inner: WithOtherFields<TransactionRequest>) -> Self {
        Self(inner)
    }

    /// Check if this is a deposit transaction.
    #[inline]
    pub fn is_deposit(&self) -> bool {
        self.as_ref().transaction_type == Some(DEPOSIT_TX_TYPE_ID)
    }

    /// Returns the minimal transaction type this request can be converted into based on the fields
    /// that are set. See [`TransactionRequest::preferred_type`].
    pub fn preferred_type(&self) -> FoundryTxType {
        if self.is_deposit() {
            FoundryTxType::Deposit
        } else {
            self.as_ref().preferred_type().into()
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
        get_deposit_tx_parts(&self.as_ref().other).map(|_| ())
    }

    /// Return the tx type this request can be built as. Computed by checking
    /// the preferred type, and then checking for completeness.
    pub fn buildable_type(&self) -> Option<FoundryTxType> {
        let pref = self.preferred_type();
        match pref {
            FoundryTxType::Legacy => self.as_ref().complete_legacy().ok(),
            FoundryTxType::Eip2930 => self.as_ref().complete_2930().ok(),
            FoundryTxType::Eip1559 => self.as_ref().complete_1559().ok(),
            FoundryTxType::Eip4844 => self.as_ref().complete_4844().ok(),
            FoundryTxType::Eip7702 => self.as_ref().complete_7702().ok(),
            FoundryTxType::Deposit => self.complete_deposit().ok(),
            // TODO: implement Tempo transaction completeness check
            FoundryTxType::Tempo => None,
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
            FoundryTxType::Legacy => self.as_ref().complete_legacy(),
            FoundryTxType::Eip2930 => self.as_ref().complete_2930(),
            FoundryTxType::Eip1559 => self.as_ref().complete_1559(),
            FoundryTxType::Eip4844 => self.complete_4844(),
            FoundryTxType::Eip7702 => self.as_ref().complete_7702(),
            FoundryTxType::Deposit => self.complete_deposit(),
            // TODO: implement Tempo transaction completeness check
            FoundryTxType::Tempo => Err(vec!["tempo transaction building not yet supported"]),
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
        if let Ok(deposit_tx_parts) = get_deposit_tx_parts(&self.as_ref().other) {
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
        } else if self.as_ref().has_eip4844_fields() && self.as_ref().blob_sidecar().is_none() {
            // if request has eip4844 fields but no blob sidecar, try to build to eip4844 without
            // sidecar
            self.0
                .into_inner()
                .build_4844_without_sidecar()
                .map_err(|e| Self(e.into_value().into()))
                .map(|tx| FoundryTypedTx::Eip4844(tx.into()))
        } else {
            // Use the inner transaction request to build EthereumTypedTransaction
            let typed_tx = self.0.into_inner().build_typed_tx().map_err(|tx| Self(tx.into()))?;
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
            FoundryTypedTx::Legacy(tx) => Self(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip2930(tx) => Self(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip1559(tx) => Self(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip4844(tx) => Self(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Eip7702(tx) => Self(Into::<TransactionRequest>::into(tx).into()),
            FoundryTypedTx::Deposit(tx) => {
                let other = OtherFields::from_iter([
                    ("sourceHash", tx.source_hash.to_string().into()),
                    ("mint", tx.mint.to_string().into()),
                    ("isSystemTx", tx.is_system_transaction.to_string().into()),
                ]);
                WithOtherFields { inner: Into::<TransactionRequest>::into(tx), other }.into()
            }
            // TODO: implement conversion from Tempo transaction to request
            FoundryTypedTx::Tempo(_) => unimplemented!("tempo transaction request conversion"),
        }
    }
}

impl From<FoundryTxEnvelope> for FoundryTransactionRequest {
    fn from(tx: FoundryTxEnvelope) -> Self {
        FoundryTypedTx::from(tx).into()
    }
}

// TransactionBuilder trait implementation for FoundryNetwork
impl TransactionBuilder<FoundryNetwork> for FoundryTransactionRequest {
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

    fn complete_type(&self, ty: FoundryTxType) -> Result<(), Vec<&'static str>> {
        match ty {
            FoundryTxType::Legacy => self.as_ref().complete_legacy(),
            FoundryTxType::Eip2930 => self.as_ref().complete_2930(),
            FoundryTxType::Eip1559 => self.as_ref().complete_1559(),
            FoundryTxType::Eip4844 => self.as_ref().complete_4844(),
            FoundryTxType::Eip7702 => self.as_ref().complete_7702(),
            FoundryTxType::Deposit => self.complete_deposit(),
            // TODO: implement Tempo transaction completeness check
            FoundryTxType::Tempo => Err(vec!["tempo transaction building not yet supported"]),
        }
    }

    fn can_submit(&self) -> bool {
        self.from().is_some()
    }

    fn can_build(&self) -> bool {
        self.as_ref().can_build() || get_deposit_tx_parts(&self.as_ref().other).is_ok()
    }

    fn output_tx_type(&self) -> FoundryTxType {
        self.preferred_type()
    }

    fn output_tx_type_checked(&self) -> Option<FoundryTxType> {
        self.buildable_type()
    }

    /// Prepares [`FoundryTransactionRequest`] by trimming conflicting fields, and filling with
    /// default values the mandatory fields.
    fn prep_for_submission(&mut self) {
        let preferred_type = self.preferred_type();
        let inner = self.as_mut();
        inner.transaction_type = Some(preferred_type as u8);
        inner.gas_limit().is_none().then(|| inner.set_gas_limit(Default::default()));
        if preferred_type != FoundryTxType::Deposit {
            inner.trim_conflicting_keys();
            inner.populate_blob_hashes();
            inner.nonce().is_none().then(|| inner.set_nonce(Default::default()));
        }
        if matches!(preferred_type, FoundryTxType::Legacy | FoundryTxType::Eip2930) {
            inner.gas_price().is_none().then(|| inner.set_gas_price(Default::default()));
        }
        if preferred_type == FoundryTxType::Eip2930 {
            inner.access_list().is_none().then(|| inner.set_access_list(Default::default()));
        }
        if matches!(
            preferred_type,
            FoundryTxType::Eip1559 | FoundryTxType::Eip4844 | FoundryTxType::Eip7702
        ) {
            inner
                .max_priority_fee_per_gas()
                .is_none()
                .then(|| inner.set_max_priority_fee_per_gas(Default::default()));
            inner
                .max_fee_per_gas()
                .is_none()
                .then(|| inner.set_max_fee_per_gas(Default::default()));
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
