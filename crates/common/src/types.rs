//! Temporary utility conversion traits between ethers-rs and alloy types.

use alloy_primitives::{Address, Bloom, Bytes, B256, B64, I256, U128, U256, U64};
use alloy_rpc_types::{
    other::OtherFields,
    request::{TransactionInput, TransactionRequest as CallRequest},
    AccessList, AccessListItem, Signature, Transaction,
};
use alloy_signer::{LocalWallet, Signer};
use ethers_core::types::{
    transaction::eip2930::{
        AccessList as EthersAccessList, AccessListItem as EthersAccessListItem,
    },
    Bloom as EthersBloom, Bytes as EthersBytes, TransactionRequest, H160, H256, H64,
    I256 as EthersI256, U256 as EthersU256, U64 as EthersU64,
};

/// Conversion trait to easily convert from Ethers types to Alloy types.
pub trait ToAlloy {
    /// The corresponding Alloy type.
    type To;

    /// Converts the Ethers type to the corresponding Alloy type.
    fn to_alloy(self) -> Self::To;
}

impl ToAlloy for EthersBytes {
    type To = Bytes;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        Bytes(self.0)
    }
}

impl ToAlloy for H64 {
    type To = B64;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        B64::new(self.0)
    }
}

impl ToAlloy for H160 {
    type To = Address;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        Address::new(self.0)
    }
}

impl ToAlloy for H256 {
    type To = B256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        B256::new(self.0)
    }
}

impl ToAlloy for EthersBloom {
    type To = Bloom;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        Bloom::new(self.0)
    }
}

impl ToAlloy for EthersU256 {
    type To = U256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        U256::from_limbs(self.0)
    }
}

impl ToAlloy for EthersI256 {
    type To = I256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        I256::from_raw(self.into_raw().to_alloy())
    }
}

impl ToAlloy for EthersU64 {
    type To = U64;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        U64::from_limbs(self.0)
    }
}

impl ToAlloy for u64 {
    type To = U256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        U256::from(self)
    }
}

impl ToAlloy for ethers_core::types::Transaction {
    type To = Transaction;

    fn to_alloy(self) -> Self::To {
        Transaction {
            hash: self.hash.to_alloy(),
            nonce: U64::from(self.nonce.as_u64()),
            block_hash: self.block_hash.map(ToAlloy::to_alloy),
            block_number: self.block_number.map(|b| U256::from(b.as_u64())),
            transaction_index: self.transaction_index.map(|b| U256::from(b.as_u64())),
            from: self.from.to_alloy(),
            to: self.to.map(ToAlloy::to_alloy),
            value: self.value.to_alloy(),
            gas_price: self.gas_price.map(|a| U128::from(a.as_u128())),
            gas: self.gas.to_alloy(),
            max_fee_per_gas: self.max_fee_per_gas.map(|f| U128::from(f.as_u128())),
            max_priority_fee_per_gas: self
                .max_priority_fee_per_gas
                .map(|f| U128::from(f.as_u128())),
            max_fee_per_blob_gas: None,
            input: self.input.0.into(),
            signature: Some(Signature {
                r: self.r.to_alloy(),
                s: self.s.to_alloy(),
                v: U256::from(self.v.as_u64()),
                y_parity: None,
            }),
            chain_id: self.chain_id.map(|c| U64::from(c.as_u64())),
            blob_versioned_hashes: Vec::new(),
            access_list: self.access_list.map(|a| a.0.into_iter().map(ToAlloy::to_alloy).collect()),
            transaction_type: self.transaction_type.map(|t| t.to_alloy()),
            other: Default::default(),
        }
    }
}

impl ToEthers for alloy_signer::LocalWallet {
    type To = ethers_signers::LocalWallet;

    fn to_ethers(self) -> Self::To {
        ethers_signers::LocalWallet::new_with_signer(
            self.signer().clone(),
            self.address().to_ethers(),
            self.chain_id().unwrap(),
        )
    }
}

impl ToEthers for Vec<LocalWallet> {
    type To = Vec<ethers_signers::LocalWallet>;

    fn to_ethers(self) -> Self::To {
        self.into_iter().map(ToEthers::to_ethers).collect()
    }
}

/// Converts from a [TransactionRequest] to a [CallRequest].
pub fn to_call_request_from_tx_request(tx: TransactionRequest) -> CallRequest {
    CallRequest {
        from: tx.from.map(|f| f.to_alloy()),
        to: match tx.to {
            Some(to) => match to {
                ethers_core::types::NameOrAddress::Address(addr) => Some(addr.to_alloy()),
                ethers_core::types::NameOrAddress::Name(_) => None,
            },
            None => None,
        },
        gas_price: tx.gas_price.map(|g| g.to_alloy()),
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        gas: tx.gas.map(|g| g.to_alloy()),
        value: tx.value.map(|v| v.to_alloy()),
        input: TransactionInput::maybe_input(tx.data.map(|b| b.0.into())),
        nonce: tx.nonce.map(|n| U64::from(n.as_u64())),
        chain_id: tx.chain_id.map(|c| c.to_alloy()),
        access_list: None,
        max_fee_per_blob_gas: None,
        blob_versioned_hashes: None,
        transaction_type: None,
        sidecar: None,
        other: OtherFields::default(),
    }
}

impl ToAlloy for EthersAccessList {
    type To = AccessList;
    fn to_alloy(self) -> Self::To {
        AccessList(self.0.into_iter().map(ToAlloy::to_alloy).collect())
    }
}

impl ToAlloy for EthersAccessListItem {
    type To = AccessListItem;

    fn to_alloy(self) -> Self::To {
        AccessListItem {
            address: self.address.to_alloy(),
            storage_keys: self.storage_keys.into_iter().map(ToAlloy::to_alloy).collect(),
        }
    }
}

/// Conversion trait to easily convert from Alloy types to Ethers types.
pub trait ToEthers {
    /// The corresponding Ethers type.
    type To;

    /// Converts the Alloy type to the corresponding Ethers type.
    fn to_ethers(self) -> Self::To;
}

impl ToEthers for Address {
    type To = H160;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        H160(self.0 .0)
    }
}

impl ToEthers for B256 {
    type To = H256;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        H256(self.0)
    }
}

impl ToEthers for U256 {
    type To = EthersU256;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        EthersU256(self.into_limbs())
    }
}

impl ToEthers for U64 {
    type To = EthersU64;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        EthersU64(self.into_limbs())
    }
}

impl ToEthers for Bytes {
    type To = EthersBytes;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        EthersBytes(self.0)
    }
}
