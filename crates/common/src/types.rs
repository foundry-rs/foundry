//! Temporary utility conversion traits between ethers-rs and alloy types.

use alloy_primitives::{Address, Bloom, Bytes, B256, B64, I256, U256, U64};
use alloy_rpc_types::{AccessList, AccessListItem, BlockNumberOrTag};
use alloy_signer_wallet::LocalWallet;
use ethers_core::types::{
    transaction::eip2930::{
        AccessList as EthersAccessList, AccessListItem as EthersAccessListItem,
    },
    BlockNumber, Bloom as EthersBloom, Bytes as EthersBytes, H160, H256, H64, I256 as EthersI256,
    U256 as EthersU256, U64 as EthersU64,
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

impl ToEthers for alloy_signer_wallet::LocalWallet {
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

impl ToEthers for BlockNumberOrTag {
    type To = BlockNumber;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        match self {
            BlockNumberOrTag::Number(n) => BlockNumber::Number(n.into()),
            BlockNumberOrTag::Earliest => BlockNumber::Earliest,
            BlockNumberOrTag::Latest => BlockNumber::Latest,
            BlockNumberOrTag::Pending => BlockNumber::Pending,
            BlockNumberOrTag::Finalized => BlockNumber::Finalized,
            BlockNumberOrTag::Safe => BlockNumber::Safe,
        }
    }
}
