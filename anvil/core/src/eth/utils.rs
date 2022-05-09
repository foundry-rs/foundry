use ethers_core::{
    types::{transaction::eip2930::AccessListItem, Address, U256},
    utils::{
        rlp,
        rlp::{Encodable, RlpStream},
    },
};
use foundry_evm::utils::h256_to_u256_be;

pub fn enveloped<T: Encodable>(id: u8, v: &T, s: &mut RlpStream) {
    let encoded = rlp::encode(v);
    let mut out = vec![0; 1 + encoded.len()];
    out[0] = id;
    out[1..].copy_from_slice(&encoded);
    out.rlp_append(s)
}

pub fn to_access_list(list: Vec<AccessListItem>) -> Vec<(Address, Vec<U256>)> {
    list.into_iter()
        .map(|item| (item.address, item.storage_keys.into_iter().map(h256_to_u256_be).collect()))
        .collect()
}
