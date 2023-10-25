//! Temporary utility conversion traits between ethers-rs and alloy types.

use alloy_primitives::{Address, B256, U256 as AlloyU256, U64 as AlloyU64};
use ethers_core::types::{H160, H256, U256, U64};

/// Conversion trait to easily convert from ethers-rs types to alloy primitive types.
pub trait ToAlloy {
    type To;

    /// Converts the alloy type to the corresponding ethers-rs type.
    fn to_alloy(self) -> Self::To;
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

impl ToAlloy for U256 {
    type To = AlloyU256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        AlloyU256::from_limbs(self.0)
    }
}

impl ToAlloy for U64 {
    type To = AlloyU64;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        AlloyU64::from_limbs(self.0)
    }
}

impl ToAlloy for u64 {
    type To = AlloyU256;

    #[inline(always)]
    fn to_alloy(self) -> Self::To {
        AlloyU256::from(self)
    }
}

/// Conversion trait to easily convert from alloy primitive types to ethers-rs types.
pub trait ToEthers {
    type To;

    /// Converts the alloy type to the corresponding ethers-rs type.
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

impl ToEthers for AlloyU256 {
    type To = U256;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        U256(self.into_limbs())
    }
}

impl ToEthers for AlloyU64 {
    type To = U64;

    #[inline(always)]
    fn to_ethers(self) -> Self::To {
        U64(self.into_limbs())
    }
}
