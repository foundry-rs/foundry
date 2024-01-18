use alloy_eips::eip2930::{
    AccessList as AlloyEipAccessList, AccessListItem as AlloyEipAccessListItem,
};
use alloy_primitives::{Address, U256};
use alloy_rpc_types::{AccessList as AlloyAccessList, AccessListItem as AlloyAccessListItem};

pub fn alloy_to_revm_access_list(list: Vec<AlloyAccessListItem>) -> Vec<(Address, Vec<U256>)> {
    list.into_iter()
        .map(|item| (item.address, item.storage_keys.into_iter().map(|k| k.into()).collect()))
        .collect()
}

pub fn from_eip_to_alloy_access_list(list: AlloyEipAccessList) -> AlloyAccessList {
    AlloyAccessList(
        list.0
            .into_iter()
            .map(|item| AlloyAccessListItem {
                address: item.address,
                storage_keys: item.storage_keys.into_iter().collect(),
            })
            .collect(),
    )
}

/// Translates a vec of [AlloyEipAccessListItem] to a revm style Access List.
pub fn eip_to_revm_access_list(list: Vec<AlloyEipAccessListItem>) -> Vec<(Address, Vec<U256>)> {
    list.into_iter()
        .map(|item| (item.address, item.storage_keys.into_iter().map(|k| k.into()).collect()))
        .collect()
}
