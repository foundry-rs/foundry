use alloy_network::Network;
use alloy_primitives::{Address, B256, Bytes};
use foundry_common::TransactionMaybeSigned;
use revm_inspectors::tracing::types::CallKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalContract {
    #[serde(rename = "transactionType")]
    pub call_kind: CallKind,
    pub contract_name: Option<String>,
    pub address: Address,
    pub init_code: Bytes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    bound(
        serialize = "N::TransactionRequest: Serialize, N::TxEnvelope: Serialize",
        deserialize = "N::TransactionRequest: for<'de2> Deserialize<'de2>, N::TxEnvelope: for<'de2> Deserialize<'de2>"
    )
)]
pub struct TransactionWithMetadata<N: Network> {
    pub hash: Option<B256>,
    #[serde(rename = "transactionType")]
    pub call_kind: CallKind,
    #[serde(default = "default_string")]
    pub contract_name: Option<String>,
    #[serde(default = "default_address")]
    pub contract_address: Option<Address>,
    #[serde(default = "default_string")]
    pub function: Option<String>,
    #[serde(default = "default_vec_of_strings")]
    pub arguments: Option<Vec<String>>,
    #[serde(skip)]
    pub rpc: String,
    pub transaction: TransactionMaybeSigned<N>,
    #[serde(default)]
    pub additional_contracts: Vec<AdditionalContract>,
    #[serde(default)]
    pub is_fixed_gas_limit: bool,
}

const fn default_string() -> Option<String> {
    Some(String::new())
}

const fn default_address() -> Option<Address> {
    Some(Address::ZERO)
}

const fn default_vec_of_strings() -> Option<Vec<String>> {
    Some(vec![])
}

impl<N: Network> TransactionWithMetadata<N> {
    pub fn from_tx_request(transaction: TransactionMaybeSigned<N>) -> Self {
        Self {
            transaction,
            hash: Default::default(),
            call_kind: Default::default(),
            contract_name: Default::default(),
            contract_address: Default::default(),
            function: Default::default(),
            arguments: Default::default(),
            is_fixed_gas_limit: Default::default(),
            additional_contracts: Default::default(),
            rpc: Default::default(),
        }
    }

    pub const fn tx(&self) -> &TransactionMaybeSigned<N> {
        &self.transaction
    }

    pub const fn tx_mut(&mut self) -> &mut TransactionMaybeSigned<N> {
        &mut self.transaction
    }

    pub fn is_create2(&self) -> bool {
        self.call_kind == CallKind::Create2
    }
}
