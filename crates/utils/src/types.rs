//! Temporary utility conversion traits between ethers-rs and alloy types.

use alloy_primitives::{Address, B256, U256 as AlloyU256};
use ethers_core::types::{H160, H256, U256};

/// Conversion trait to easily convert from ethers-rs types to alloy primitive types.
pub trait ToAlloy {
    type To;

    /// Converts the alloy type to the corresponding ethers-rs type.
    fn to_alloy(self) -> Self::To;
}

impl ToAlloy for H160 {
    type To = Address;

    fn to_alloy(self) -> Self::To {
        Address::from_slice(self.as_bytes())
    }
}

impl ToAlloy for H256 {
    type To = alloy_primitives::B256;

    fn to_alloy(self) -> Self::To {
        B256::from_slice(self.as_bytes())
    }
}

impl ToAlloy for U256 {
    type To = alloy_primitives::U256;

    fn to_alloy(self) -> Self::To {
        let mut buffer = [0u8; 32];
        self.to_little_endian(buffer.as_mut_slice());
        AlloyU256::from_le_bytes(buffer)
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

    fn to_ethers(self) -> Self::To {
        H160::from_slice(self.as_slice())
    }
}

impl ToEthers for B256 {
    type To = H256;

    fn to_ethers(self) -> Self::To {
        H256(self.0)
    }
}

impl ToEthers for AlloyU256 {
    type To = U256;

    fn to_ethers(self) -> Self::To {
        U256::from_little_endian(&self.as_le_bytes())
    }
}
