use ethers::prelude::{H256, U256};
use revm::{opcode, spec_opcode_gas, SpecId};
use std::collections::BTreeMap;

/// Small helper function to convert [U256] into [H256].
pub fn u256_to_h256_le(u: U256) -> H256 {
    let mut h = H256::default();
    u.to_little_endian(h.as_mut());
    h
}

/// Small helper function to convert [U256] into [H256].
pub fn u256_to_h256_be(u: U256) -> H256 {
    let mut h = H256::default();
    u.to_big_endian(h.as_mut());
    h
}

/// Small helper function to convert [H256] into [U256].
pub fn h256_to_u256_be(storage: H256) -> U256 {
    U256::from_big_endian(storage.as_bytes())
}

/// Small helper function to convert [H256] into [U256].
pub fn h256_to_u256_le(storage: H256) -> U256 {
    U256::from_little_endian(storage.as_bytes())
}

/// Builds the instruction counter map for the given bytecode.
// TODO: Some of the same logic is performed in REVM, but then later discarded. We should
// investigate if we can reuse it
pub fn build_ic_map(spec: SpecId, code: &[u8]) -> BTreeMap<usize, usize> {
    let opcode_infos = spec_opcode_gas(spec);
    let mut ic_map: BTreeMap<usize, usize> = BTreeMap::new();

    let mut i = 0;
    let mut cumulative_push_size = 0;
    while i < code.len() {
        let op = code[i];
        ic_map.insert(i, i - cumulative_push_size);
        if opcode_infos[op as usize].is_push() {
            // Skip the push bytes.
            //
            // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
            i += (op - opcode::PUSH1 + 1) as usize;
            cumulative_push_size += (op - opcode::PUSH1 + 1) as usize;
        }
        i += 1;
    }

    ic_map
}
