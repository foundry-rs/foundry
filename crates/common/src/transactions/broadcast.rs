use alloy_consensus::Transaction;
use alloy_eips::eip7702::SignedAuthorization;
use alloy_network::{Network, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::{Address, Bytes, U256};
use foundry_common_fmt::UIfmt;
use serde::{Deserialize, Serialize};

use super::FoundryTransactionBuilder;

/// Used for broadcasting transactions
/// A transaction can either be a `TransactionRequest` waiting to be signed
/// or a `TxEnvelope`, already signed
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TransactionMaybeSigned<N: Network> {
    Signed {
        #[serde(flatten)]
        tx: N::TxEnvelope,
        from: Address,
    },
    Unsigned(N::TransactionRequest),
}

impl<N: Network> TransactionMaybeSigned<N> {
    /// Creates a new (unsigned) transaction for broadcast
    pub const fn new(tx: N::TransactionRequest) -> Self {
        Self::Unsigned(tx)
    }

    pub const fn is_unsigned(&self) -> bool {
        matches!(self, Self::Unsigned(_))
    }

    pub const fn as_unsigned_mut(&mut self) -> Option<&mut N::TransactionRequest> {
        match self {
            Self::Unsigned(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn from(&self) -> Option<Address> {
        match self {
            Self::Signed { from, .. } => Some(*from),
            Self::Unsigned(tx) => tx.from(),
        }
    }

    pub fn input(&self) -> Option<&Bytes> {
        match self {
            Self::Signed { tx, .. } => Some(tx.input()),
            Self::Unsigned(tx) => tx.input(),
        }
    }

    pub fn to(&self) -> Option<Address> {
        match self {
            Self::Signed { tx, .. } => tx.to(),
            Self::Unsigned(tx) => tx.to(),
        }
    }

    pub fn value(&self) -> Option<U256> {
        match self {
            Self::Signed { tx, .. } => Some(tx.value()),
            Self::Unsigned(tx) => tx.value(),
        }
    }

    pub fn gas(&self) -> Option<u128> {
        match self {
            Self::Signed { tx, .. } => Some(tx.gas_limit() as u128),
            Self::Unsigned(tx) => tx.gas_limit().map(|g| g as u128),
        }
    }

    pub fn nonce(&self) -> Option<u64> {
        match self {
            Self::Signed { tx, .. } => Some(tx.nonce()),
            Self::Unsigned(tx) => tx.nonce(),
        }
    }

    pub fn authorization_list(&self) -> Option<Vec<SignedAuthorization>>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        match self {
            Self::Signed { tx, .. } => tx.authorization_list().map(|auths| auths.to_vec()),
            Self::Unsigned(tx) => tx.authorization_list().cloned(),
        }
        .filter(|auths| !auths.is_empty())
    }
}

impl<N: Network> UIfmt for TransactionMaybeSigned<N>
where
    N::TxEnvelope: UIfmt,
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    fn pretty(&self) -> String {
        match self {
            Self::Signed { tx, .. } => tx.pretty(),
            Self::Unsigned(tx) => format!(
                "
accessList           {}
chainId              {}
gasLimit             {}
gasPrice             {}
input                {}
maxFeePerBlobGas     {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
to                   {}
type                 {}
value                {}",
                tx.access_list()
                    .as_ref()
                    .map(|a| a.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .pretty(),
                tx.chain_id().pretty(),
                tx.gas_limit().unwrap_or_default(),
                tx.gas_price().pretty(),
                tx.input().pretty(),
                tx.max_fee_per_blob_gas().pretty(),
                tx.max_fee_per_gas().pretty(),
                tx.max_priority_fee_per_gas().pretty(),
                tx.nonce().pretty(),
                tx.to().pretty(),
                NetworkTransactionBuilder::<N>::output_tx_type(tx),
                tx.value().pretty(),
            ),
        }
    }
}
