//! [EIP-4895] Withdrawal type and serde helpers.
//!
//! [EIP-4895]: https://eips.ethereum.org/EIPS/eip-4895

use alloc::vec::Vec;
use alloy_primitives::{Address, U256};
use alloy_rlp::{RlpDecodable, RlpDecodableWrapper, RlpEncodable, RlpEncodableWrapper};
use derive_more::derive::{AsRef, Deref, DerefMut, From, IntoIterator};

/// Multiplier for converting gwei to wei.
pub const GWEI_TO_WEI: u64 = 1_000_000_000;

/// Withdrawal represents a validator withdrawal from the consensus layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, RlpEncodable, RlpDecodable)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ssz", derive(ssz_derive::Encode, ssz_derive::Decode))]
pub struct Withdrawal {
    /// Monotonically increasing identifier issued by consensus layer.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub index: u64,
    /// Index of validator associated with withdrawal.
    #[cfg_attr(
        feature = "serde",
        serde(with = "alloy_serde::quantity", rename = "validatorIndex")
    )]
    pub validator_index: u64,
    /// Target address for withdrawn ether.
    pub address: Address,
    /// Value of the withdrawal in gwei.
    #[cfg_attr(feature = "serde", serde(with = "alloy_serde::quantity"))]
    pub amount: u64,
}

impl Withdrawal {
    /// Return the withdrawal amount in wei.
    pub fn amount_wei(&self) -> U256 {
        U256::from(self.amount) * U256::from(GWEI_TO_WEI)
    }
}

/// Represents a collection of Withdrawals.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Default,
    Hash,
    From,
    AsRef,
    Deref,
    DerefMut,
    IntoIterator,
    RlpEncodableWrapper,
    RlpDecodableWrapper,
)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Withdrawals(pub Vec<Withdrawal>);

impl Withdrawals {
    /// Create a new Withdrawals instance.
    pub const fn new(withdrawals: Vec<Withdrawal>) -> Self {
        Self(withdrawals)
    }

    /// Calculate the total size, including capacity, of the Withdrawals.
    #[inline]
    pub fn total_size(&self) -> usize {
        self.0.capacity() * core::mem::size_of::<Withdrawal>()
    }

    /// Calculate a heuristic for the in-memory size of the [Withdrawals].
    #[inline]
    pub fn size(&self) -> usize {
        self.0.len() * core::mem::size_of::<Withdrawal>()
    }

    /// Get an iterator over the Withdrawals.
    pub fn iter(&self) -> core::slice::Iter<'_, Withdrawal> {
        self.0.iter()
    }

    /// Get a mutable iterator over the Withdrawals.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, Withdrawal> {
        self.0.iter_mut()
    }

    /// Convert [Self] into raw vec of withdrawals.
    pub fn into_inner(self) -> Vec<Withdrawal> {
        self.0
    }
}

impl<'a> IntoIterator for &'a Withdrawals {
    type Item = &'a Withdrawal;
    type IntoIter = core::slice::Iter<'a, Withdrawal>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Withdrawals {
    type Item = &'a mut Withdrawal;
    type IntoIter = core::slice::IterMut<'a, Withdrawal>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;
    use core::str::FromStr;

    // <https://github.com/paradigmxyz/reth/issues/1614>
    #[test]
    fn test_withdrawal_serde_roundtrip() {
        let input = r#"[{"index":"0x0","validatorIndex":"0x0","address":"0x0000000000000000000000000000000000001000","amount":"0x1"},{"index":"0x1","validatorIndex":"0x1","address":"0x0000000000000000000000000000000000001001","amount":"0x1"},{"index":"0x2","validatorIndex":"0x2","address":"0x0000000000000000000000000000000000001002","amount":"0x1"},{"index":"0x3","validatorIndex":"0x3","address":"0x0000000000000000000000000000000000001003","amount":"0x1"},{"index":"0x4","validatorIndex":"0x4","address":"0x0000000000000000000000000000000000001004","amount":"0x1"},{"index":"0x5","validatorIndex":"0x5","address":"0x0000000000000000000000000000000000001005","amount":"0x1"},{"index":"0x6","validatorIndex":"0x6","address":"0x0000000000000000000000000000000000001006","amount":"0x1"},{"index":"0x7","validatorIndex":"0x7","address":"0x0000000000000000000000000000000000001007","amount":"0x1"},{"index":"0x8","validatorIndex":"0x8","address":"0x0000000000000000000000000000000000001008","amount":"0x1"},{"index":"0x9","validatorIndex":"0x9","address":"0x0000000000000000000000000000000000001009","amount":"0x1"},{"index":"0xa","validatorIndex":"0xa","address":"0x000000000000000000000000000000000000100a","amount":"0x1"},{"index":"0xb","validatorIndex":"0xb","address":"0x000000000000000000000000000000000000100b","amount":"0x1"},{"index":"0xc","validatorIndex":"0xc","address":"0x000000000000000000000000000000000000100c","amount":"0x1"},{"index":"0xd","validatorIndex":"0xd","address":"0x000000000000000000000000000000000000100d","amount":"0x1"},{"index":"0xe","validatorIndex":"0xe","address":"0x000000000000000000000000000000000000100e","amount":"0x1"},{"index":"0xf","validatorIndex":"0xf","address":"0x000000000000000000000000000000000000100f","amount":"0x1"}]"#;

        // With a vector of withdrawals.
        let withdrawals: Vec<Withdrawal> = serde_json::from_str(input).unwrap();
        let s = serde_json::to_string(&withdrawals).unwrap();
        assert_eq!(input, s);

        // With a Withdrawals struct.
        let withdrawals: Withdrawals = serde_json::from_str(input).unwrap();
        let s = serde_json::to_string(&withdrawals).unwrap();
        assert_eq!(input, s);
    }

    #[test]
    fn test_withdrawal_amount_wei() {
        let withdrawal =
            Withdrawal { index: 1, validator_index: 2, address: Address::random(), amount: 454456 };

        // Assert that the amount_wei method returns the correct value
        assert_eq!(withdrawal.amount_wei(), U256::from_str("0x19d5348723000").unwrap());
    }
}
