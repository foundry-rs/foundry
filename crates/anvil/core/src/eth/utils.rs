use alloy_eips::eip2930::AccessListItem as AlloyEipAccessListItem;
use alloy_primitives::{Address, U256};
use alloy_rpc_types::AccessListItem as AlloyAccessListItem;
use ethers_core::{
    types::transaction::eip2930::AccessListItem,
    utils::{
        rlp,
        rlp::{Encodable, RlpStream},
    },
};
use foundry_common::types::ToAlloy;

pub fn enveloped<T: Encodable>(id: u8, v: &T, s: &mut RlpStream) {
    let encoded = rlp::encode(v);
    let mut out = vec![0; 1 + encoded.len()];
    out[0] = id;
    out[1..].copy_from_slice(&encoded);
    out.rlp_append(s)
}

pub fn to_revm_access_list(list: Vec<AccessListItem>) -> Vec<(Address, Vec<U256>)> {
    list.into_iter()
        .map(|item| {
            (
                item.address.to_alloy(),
                item.storage_keys.into_iter().map(|k| k.to_alloy().into()).collect(),
            )
        })
        .collect()
}

pub fn alloy_to_revm_access_list(list: Vec<AlloyAccessListItem>) -> Vec<(Address, Vec<U256>)> {
    list.into_iter()
        .map(|item| (item.address, item.storage_keys.into_iter().map(|k| k.into()).collect()))
        .collect()
}

pub fn eip_to_revm_access_list(list: Vec<AlloyEipAccessListItem>) -> Vec<(Address, Vec<U256>)> {
    list.into_iter()
        .map(|item| (item.address, item.storage_keys.into_iter().map(|k| k.into()).collect()))
        .collect()
}
