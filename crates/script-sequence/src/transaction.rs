use alloy_primitives::{Address, B256, Bytes};
use foundry_common::TransactionMaybeSigned;
use revm_inspectors::tracing::types::CallKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdditionalContract {
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
    pub contract_name: Option<String>,
    pub address: Address,
    pub init_code: Bytes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithMetadata {
    pub hash: Option<B256>,
    #[serde(rename = "transactionType")]
    pub opcode: CallKind,
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
    pub transaction: TransactionMaybeSigned,
    pub additional_contracts: Vec<AdditionalContract>,
    pub is_fixed_gas_limit: bool,
}

fn default_string() -> Option<String> {
    Some(String::new())
}

fn default_address() -> Option<Address> {
    Some(Address::ZERO)
}

fn default_vec_of_strings() -> Option<Vec<String>> {
    Some(vec![])
}

impl TransactionWithMetadata {
    pub fn from_tx_request(transaction: TransactionMaybeSigned) -> Self {
        Self {
            transaction,
            hash: Default::default(),
            opcode: Default::default(),
            contract_name: Default::default(),
            contract_address: Default::default(),
            function: Default::default(),
            arguments: Default::default(),
            is_fixed_gas_limit: Default::default(),
            additional_contracts: Default::default(),
            rpc: Default::default(),
        }
    }

    pub fn tx(&self) -> &TransactionMaybeSigned {
        &self.transaction
    }

    pub fn tx_mut(&mut self) -> &mut TransactionMaybeSigned {
        &mut self.transaction
    }

    pub fn is_create2(&self) -> bool {
        self.opcode == CallKind::Create2
    }
}
