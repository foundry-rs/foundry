use alloy_eips::eip2930::{
    AccessList as AlloyEipAccessList, AccessListItem as AlloyEipAccessListItem,
};
use alloy_primitives::{Address, Parity, U256};
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

/// See <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
/// > If you do, then the v of the signature MUST be set to {0,1} + CHAIN_ID * 2 + 35 where
/// > {0,1} is the parity of the y value of the curve point for which r is the x-value in the
/// > secp256k1 signing process.
pub fn meets_eip155(chain_id: u64, v: Parity) -> bool {
    let double_chain_id = chain_id.saturating_mul(2);
    match v {
        Parity::Eip155(v) => v == double_chain_id + 35 || v == double_chain_id + 36,
        _ => false,
    }
}
