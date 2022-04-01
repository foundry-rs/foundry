use ethers::prelude::{H256, U256};

/// Small helper function to convert [U256] into [H256].
pub fn u256_to_h256_le(u: U256) -> H256 {
    let mut h = H256::default();
    u.to_little_endian(h.as_mut());
    h
}
