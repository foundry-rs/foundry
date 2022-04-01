use ethers_core::utils::{
    rlp,
    rlp::{Encodable, RlpStream},
};

pub fn enveloped<T: Encodable>(id: u8, v: &T, s: &mut RlpStream) {
    let encoded = rlp::encode(v);
    let mut out = vec![0; 1 + encoded.len()];
    out[0] = id;
    out[1..].copy_from_slice(&encoded);
    out.rlp_append(s)
}
