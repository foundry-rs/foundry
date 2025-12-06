use std::ops::{Deref, DerefMut};

use alloy_consensus::constants::{
    EIP1559_TX_TYPE_ID, EIP2930_TX_TYPE_ID, EIP4844_TX_TYPE_ID, EIP7702_TX_TYPE_ID,
    LEGACY_TX_TYPE_ID,
};
use alloy_network::{BuildResult, NetworkWallet, TransactionBuilder, TransactionBuilderError};
use alloy_primitives::{Address, ChainId, TxKind, U256};
use alloy_rpc_types::{AccessList, TransactionInputKind, TransactionRequest};
use alloy_serde::WithOtherFields;
use op_alloy_consensus::{DEPOSIT_TX_TYPE_ID, OpTypedTransaction};
use op_alloy_rpc_types::OpTransactionRequest;
use serde::{Deserialize, Serialize};

use super::{FoundryTxEnvelope, FoundryTxType, FoundryTypedTx};
use crate::FoundryNetwork;

/// Foundry transaction request builder.
///
/// This is implemented as a transparent wrapper around [`OpTransactionRequest`],
/// which provides handling of deposit transactions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoundryTransactionRequest(OpTransactionRequest);

impl Deref for FoundryTransactionRequest {
    type Target = OpTransactionRequest;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for FoundryTransactionRequest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Serialize for FoundryTransactionRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FoundryTransactionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        OpTransactionRequest::deserialize(deserializer).map(Self)
    }
}

impl From<OpTransactionRequest> for FoundryTransactionRequest {
    fn from(req: OpTransactionRequest) -> Self {
        Self(req)
    }
}

impl From<FoundryTransactionRequest> for OpTransactionRequest {
    fn from(req: FoundryTransactionRequest) -> Self {
        req.0
    }
}

impl From<TransactionRequest> for FoundryTransactionRequest {
    fn from(req: TransactionRequest) -> Self {
        Self(req.into())
    }
}

impl From<FoundryTransactionRequest> for TransactionRequest {
    fn from(req: FoundryTransactionRequest) -> Self {
        req.0.into()
    }
}

impl From<WithOtherFields<TransactionRequest>> for FoundryTransactionRequest {
    fn from(req: WithOtherFields<TransactionRequest>) -> Self {
        Self(req.inner.into())
    }
}

impl FoundryTransactionRequest {
    /// Create a new empty transaction request.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if this is a deposit transaction.
    #[inline]
    pub fn is_deposit(&self) -> bool {
        self.as_ref().transaction_type == Some(0x7E)
    }

    /// Mark this as a deposit transaction.
    #[inline]
    pub fn set_deposit_type(&mut self) {
        self.as_mut().transaction_type = Some(0x7E);
    }

    /// Builder-pattern method to mark as deposit transaction.
    #[inline]
    pub fn as_deposit(mut self) -> Self {
        self.set_deposit_type();
        self
    }

    /// Build a typed transaction from this request.
    ///
    /// Converts the request into a `FoundryTypedTx`, handling all Ethereum
    /// and OP-stack transaction types.
    pub fn build_typed_tx(self) -> Result<FoundryTypedTx, Self> {
        // Use OpTransactionRequest's build_typed_tx
        let op_typed = self.0.build_typed_tx().map_err(Self)?;

        // Convert OpTypedTransaction → FoundryTypedTx
        Ok(match op_typed {
            OpTypedTransaction::Legacy(tx) => FoundryTypedTx::Legacy(tx),
            OpTypedTransaction::Eip2930(tx) => FoundryTypedTx::Eip2930(tx),
            OpTypedTransaction::Eip1559(tx) => FoundryTypedTx::Eip1559(tx),
            OpTypedTransaction::Eip7702(tx) => FoundryTypedTx::Eip7702(tx),
            OpTypedTransaction::Deposit(tx) => FoundryTypedTx::Deposit(tx),
        })
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
            FoundryTypedTx::Deposit(tx) => Self(tx.into()),
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
            FoundryTxType::Legacy => {
                let mut missing = Vec::new();
                if self.from().is_none() {
                    missing.push("from");
                }
                if self.gas_limit().is_none() {
                    missing.push("gas");
                }
                if self.gas_price().is_none() {
                    missing.push("gasPrice");
                }
                if missing.is_empty() { Ok(()) } else { Err(missing) }
            }
            FoundryTxType::Eip2930 => {
                let mut missing = Vec::new();
                if self.from().is_none() {
                    missing.push("from");
                }
                if self.gas_limit().is_none() {
                    missing.push("gas");
                }
                if self.gas_price().is_none() {
                    missing.push("gasPrice");
                }
                if self.access_list().is_none() {
                    missing.push("accessList");
                }
                if missing.is_empty() { Ok(()) } else { Err(missing) }
            }
            FoundryTxType::Eip1559 => {
                let mut missing = Vec::new();
                if self.from().is_none() {
                    missing.push("from");
                }
                if self.gas_limit().is_none() {
                    missing.push("gas");
                }
                if self.max_fee_per_gas().is_none() {
                    missing.push("maxFeePerGas");
                }
                if self.max_priority_fee_per_gas().is_none() {
                    missing.push("maxPriorityFeePerGas");
                }
                if missing.is_empty() { Ok(()) } else { Err(missing) }
            }
            FoundryTxType::Eip4844 => {
                let mut missing = Vec::new();
                if self.from().is_none() {
                    missing.push("from");
                }
                if self.kind().is_none() {
                    missing.push("to");
                }
                if self.gas_limit().is_none() {
                    missing.push("gas");
                }
                if self.max_fee_per_gas().is_none() {
                    missing.push("maxFeePerGas");
                }
                if self.max_priority_fee_per_gas().is_none() {
                    missing.push("maxPriorityFeePerGas");
                }
                if self.as_ref().sidecar.is_none() {
                    missing.push("blobVersionedHashes or sidecar");
                }
                if missing.is_empty() { Ok(()) } else { Err(missing) }
            }
            FoundryTxType::Eip7702 => {
                let mut missing = Vec::new();
                if self.from().is_none() {
                    missing.push("from");
                }
                if self.kind().is_none() {
                    missing.push("to");
                }
                if self.gas_limit().is_none() {
                    missing.push("gas");
                }
                if self.max_fee_per_gas().is_none() {
                    missing.push("maxFeePerGas");
                }
                if self.max_priority_fee_per_gas().is_none() {
                    missing.push("maxPriorityFeePerGas");
                }
                if self.as_ref().authorization_list.is_none() {
                    missing.push("authorizationList");
                }
                if missing.is_empty() { Ok(()) } else { Err(missing) }
            }
            FoundryTxType::Deposit => {
                let mut missing = Vec::new();
                if self.from().is_none() {
                    missing.push("from");
                }
                if self.kind().is_none() {
                    missing.push("to");
                }
                if missing.is_empty() { Ok(()) } else { Err(missing) }
            }
        }
    }

    fn can_submit(&self) -> bool {
        self.from().is_some()
    }

    fn can_build(&self) -> bool {
        let inner = self.as_ref();
        let common = inner.gas.is_some() && inner.nonce.is_some();

        let legacy = inner.gas_price.is_some();
        let eip2930 = legacy && inner.access_list.is_some();
        let eip1559 = inner.max_fee_per_gas.is_some() && inner.max_priority_fee_per_gas.is_some();
        let eip4844 = eip1559 && inner.sidecar.is_some() && inner.to.is_some();
        let eip7702 = eip1559 && inner.authorization_list.is_some();

        let deposit =
            inner.transaction_type == Some(0x7E) && inner.from.is_some() && inner.to.is_some();

        (common && (legacy || eip2930 || eip1559 || eip4844 || eip7702)) || deposit
    }

    fn output_tx_type(&self) -> FoundryTxType {
        if self.as_ref().transaction_type == Some(DEPOSIT_TX_TYPE_ID) {
            return FoundryTxType::Deposit;
        }

        // Default to EIP1559 if the transaction type is not explicitly set
        if self.as_ref().transaction_type.is_some() {
            match self.as_ref().transaction_type.unwrap() {
                LEGACY_TX_TYPE_ID => FoundryTxType::Legacy,
                EIP2930_TX_TYPE_ID => FoundryTxType::Eip2930,
                EIP1559_TX_TYPE_ID => FoundryTxType::Eip1559,
                EIP4844_TX_TYPE_ID => FoundryTxType::Eip4844,
                EIP7702_TX_TYPE_ID => FoundryTxType::Eip7702,
                _ => FoundryTxType::Eip1559,
            }
        } else if self.max_fee_per_gas().is_some() {
            FoundryTxType::Eip1559
        } else if self.gas_price().is_some() {
            FoundryTxType::Legacy
        } else {
            FoundryTxType::Eip1559
        }
    }

    fn output_tx_type_checked(&self) -> Option<FoundryTxType> {
        if self.can_build() { Some(self.output_tx_type()) } else { None }
    }

    fn prep_for_submission(&mut self) {
        // Set transaction type if not already set
        if self.as_ref().transaction_type.is_none() {
            self.as_mut().transaction_type = Some(match self.output_tx_type() {
                FoundryTxType::Legacy => LEGACY_TX_TYPE_ID,
                FoundryTxType::Eip2930 => EIP2930_TX_TYPE_ID,
                FoundryTxType::Eip1559 => EIP1559_TX_TYPE_ID,
                FoundryTxType::Eip4844 => EIP4844_TX_TYPE_ID,
                FoundryTxType::Eip7702 => EIP7702_TX_TYPE_ID,
                FoundryTxType::Deposit => DEPOSIT_TX_TYPE_ID,
            });
        }
    }

    fn build_unsigned(self) -> BuildResult<FoundryTypedTx, FoundryNetwork> {
        // Try to build the transaction
        match self.build_typed_tx() {
            Ok(tx) => Ok(tx),
            Err(err_self) => {
                // If build_typed_tx fails, it's likely missing required fields
                let tx_type = err_self.output_tx_type();
                let mut missing = Vec::new();
                if let Err(m) = err_self.complete_type(tx_type) {
                    missing.extend(m);
                }
                // Return a generic error with missing field information
                Err(TransactionBuilderError::InvalidTransactionRequest(tx_type, missing)
                    .into_unbuilt(err_self))
            }
        }
    }

    async fn build<W: NetworkWallet<FoundryNetwork>>(
        self,
        wallet: &W,
    ) -> Result<FoundryTxEnvelope, TransactionBuilderError<FoundryNetwork>> {
        Ok(wallet.sign_request(self).await?)
    }
}
