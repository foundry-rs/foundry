//! [EIP-2930] types.
//!
//! [EIP-2930]: https://eips.ethereum.org/EIPS/eip-2930
#![cfg_attr(not(feature = "std"), no_std)]

#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use alloy_primitives::{Address, B256, U256};
use alloy_rlp::{RlpDecodable, RlpDecodableWrapper, RlpEncodable, RlpEncodableWrapper};
use core::{mem, ops::Deref};
/// A list of addresses and storage keys that the transaction plans to access.
/// Accesses outside the list are possible, but become more expensive.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, RlpDecodable, RlpEncodable)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct AccessListItem {
    /// Account addresses that would be loaded at the start of execution
    pub address: Address,
    /// Keys of storage that would be loaded at the start of execution
    pub storage_keys: Vec<B256>,
}

impl AccessListItem {
    /// Calculates a heuristic for the in-memory size of the [AccessListItem].
    #[inline]
    pub fn size(&self) -> usize {
        mem::size_of::<Address>() + self.storage_keys.capacity() * mem::size_of::<B256>()
    }
}

/// AccessList as defined in EIP-2930
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, RlpDecodableWrapper, RlpEncodableWrapper)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AccessList(pub Vec<AccessListItem>);

impl From<Vec<AccessListItem>> for AccessList {
    fn from(list: Vec<AccessListItem>) -> Self {
        Self(list)
    }
}

impl From<AccessList> for Vec<AccessListItem> {
    fn from(this: AccessList) -> Self {
        this.0
    }
}

impl Deref for AccessList {
    type Target = Vec<AccessListItem>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AccessList {
    /// Converts the list into a vec, expected by revm
    pub fn flattened(&self) -> Vec<(Address, Vec<U256>)> {
        self.flatten().collect()
    }

    /// Consumes the type and converts the list into a vec, expected by revm
    pub fn into_flattened(self) -> Vec<(Address, Vec<U256>)> {
        self.into_flatten().collect()
    }

    /// Consumes the type and returns an iterator over the list's addresses and storage keys.
    pub fn into_flatten(self) -> impl Iterator<Item = (Address, Vec<U256>)> {
        self.0.into_iter().map(|item| {
            (
                item.address,
                item.storage_keys.into_iter().map(|slot| U256::from_be_bytes(slot.0)).collect(),
            )
        })
    }

    /// Returns an iterator over the list's addresses and storage keys.
    pub fn flatten(&self) -> impl Iterator<Item = (Address, Vec<U256>)> + '_ {
        self.0.iter().map(|item| {
            (
                item.address,
                item.storage_keys.iter().map(|slot| U256::from_be_bytes(slot.0)).collect(),
            )
        })
    }

    /// Returns the position of the given address in the access list, if present.
    fn index_of_address(&self, address: Address) -> Option<usize> {
        self.iter().position(|item| item.address == address)
    }

    /// Checks if a specific storage slot within an account is present in the access list.
    ///
    /// Returns a tuple with flags for the presence of the account and the slot.
    pub fn contains_storage(&self, address: Address, slot: B256) -> (bool, bool) {
        self.index_of_address(address)
            .map_or((false, false), |idx| (true, self.contains_storage_key_at_index(slot, idx)))
    }

    /// Checks if the access list contains the specified address.
    pub fn contains_address(&self, address: Address) -> bool {
        self.iter().any(|item| item.address == address)
    }

    /// Checks if the storage keys at the given index within an account are present in the access
    /// list.
    fn contains_storage_key_at_index(&self, slot: B256, index: usize) -> bool {
        self.get(index).map_or(false, |entry| {
            entry.storage_keys.iter().any(|storage_key| *storage_key == slot)
        })
    }

    /// Adds an address to the access list and returns `true` if the operation results in a change,
    /// indicating that the address was not previously present.
    pub fn add_address(&mut self, address: Address) -> bool {
        !self.contains_address(address) && {
            self.0.push(AccessListItem { address, storage_keys: Vec::new() });
            true
        }
    }

    /// Calculates a heuristic for the in-memory size of the [AccessList].
    #[inline]
    pub fn size(&self) -> usize {
        // take into account capacity
        self.0.iter().map(AccessListItem::size).sum::<usize>()
            + self.0.capacity() * mem::size_of::<AccessListItem>()
    }
}

/// Access list with gas used appended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct AccessListWithGasUsed {
    /// List with accounts accessed during transaction.
    pub access_list: AccessList,
    /// Estimated gas used with access list.
    pub gas_used: U256,
}

/// `AccessListResult` for handling errors from `eth_createAccessList`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct AccessListResult {
    /// List with accounts accessed during transaction.
    pub access_list: AccessList,
    /// Estimated gas used with access list.
    pub gas_used: U256,
    /// Optional error message if the transaction failed.
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub error: Option<String>,
}

impl AccessListResult {
    /// Ensures the result is OK, returning [`AccessListWithGasUsed`] if so, or an error message if
    /// not.
    pub fn ensure_ok(self) -> Result<AccessListWithGasUsed, String> {
        match self.error {
            Some(err) => Err(err),
            None => {
                Ok(AccessListWithGasUsed { access_list: self.access_list, gas_used: self.gas_used })
            }
        }
    }

    /// Checks if there is an error in the result.
    #[inline]
    pub const fn is_err(&self) -> bool {
        self.error.is_some()
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    #[test]
    fn access_list_serde() {
        let list = AccessList(vec![
            AccessListItem { address: Address::ZERO, storage_keys: vec![B256::ZERO] },
            AccessListItem { address: Address::ZERO, storage_keys: vec![B256::ZERO] },
        ]);
        let json = serde_json::to_string(&list).unwrap();
        let list2 = serde_json::from_str::<AccessList>(&json).unwrap();
        assert_eq!(list, list2);
    }

    #[test]
    fn access_list_with_gas_used() {
        let list = AccessListResult {
            access_list: AccessList(vec![
                AccessListItem { address: Address::ZERO, storage_keys: vec![B256::ZERO] },
                AccessListItem { address: Address::ZERO, storage_keys: vec![B256::ZERO] },
            ]),
            gas_used: U256::from(100),
            error: None,
        };
        let json = serde_json::to_string(&list).unwrap();
        let list2 = serde_json::from_str(&json).unwrap();
        assert_eq!(list, list2);
    }
}
