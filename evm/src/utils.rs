use ethers::{
    abi::{Abi, FixedBytes, Function},
    prelude::{H256, U256},
    types::{Block, Chain},
};
use eyre::ContextCompat;
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

/// Small helper function to convert revm's [B160] into ethers's [H160].
pub fn b160_to_h160(b: revm::primitives::B160) -> ethers::types::H160 {
    ethers::types::H160::from_slice(&b.to_fixed_bytes())
}

/// Small helper function to convert ethers's [H160] into revm's [B160].
pub fn h160_to_b160(h: ethers::types::H160) -> revm::primitives::B160 {
    revm::primitives::B160::from_slice(&h.to_fixed_bytes())
}

/// Small helper function to convert revm's [B256] into ethers's [H256].
pub fn b256_to_h256(b: revm::primitives::B256) -> ethers::types::H256 {
    ethers::types::H256::from_slice(&b.to_fixed_bytes())
}

/// Small helper function to convert ether's [H256] into revm's [B256].
pub fn h256_to_b256(h: ethers::types::H256) -> revm::primitives::B256 {
    revm::primitives::B256::from_slice(&h.to_fixed_bytes())
}

/// Small helper function to convert ether's [U256] into revm's [U256].
pub fn u256_to_ru256(u: ethers::types::U256) -> revm::primitives::U256 {
    let mut buffer = [0u8; 32];
    u.to_little_endian(buffer);
    revm::primitives::U256::from_le_bytes(buffer)
}

/// Small helper function to convert revm's [U256] into ethers's [U256].
pub fn ru256_to_u256(u: revm::primitives::U256) -> ethers::types::U256 {
    ethers::types::U256::from_little_endian(u.as_le_slice())
}

/// Depending on the configured chain id and block number this should apply any specific changes
///
/// This checks for:
///    - prevrandao mixhash after merge
pub fn apply_chain_and_block_specific_env_changes<T>(env: &mut revm::primitives::Env, block: &Block<T>) {
    if let Ok(chain) = Chain::try_from(U256::from_little_endian(&env.cfg.chain_id.to_le_bytes())) {
        let block_number = block.number.unwrap_or_default();

        match chain {
            Chain::Mainnet => {
                // after merge difficulty is supplanted with prevrandao EIP-4399
                if block_number.as_u64() >= 15_537_351u64 {
                    env.block.difficulty = revm::primitives::U256::from_be_bytes(env.block.prevrandao.unwrap_or_default().to_fixed_bytes());
                }

                return
            }
            Chain::Arbitrum |
            Chain::ArbitrumGoerli |
            Chain::ArbitrumNova |
            Chain::ArbitrumTestnet => {
                // on arbitrum `block.number` is the L1 block which is included in the
                // `l1BlockNumber` field
                if let Some(l1_block_number) = block.other.get("l1BlockNumber").cloned() {
                    if let Ok(l1_block_number) = serde_json::from_value::<U256>(l1_block_number) {
                        env.block.number = l1_block_number;
                    }
                }
            }
            _ => {}
        }
    }

    // if difficulty is `0` we assume it's past merge
    if block.difficulty.is_zero() {
        env.block.difficulty = revm::primitives::U256::from_be_bytes(env.block.prevrandao.unwrap_or_default().to_fixed_bytes());
    }
}

/// A map of program counters to instruction counters.
pub type PCICMap = BTreeMap<usize, usize>;

/// Builds a mapping from program counters to instruction counters.
pub fn build_pc_ic_map(spec: SpecId, code: &[u8]) -> PCICMap {
    let opcode_infos = spec_opcode_gas(spec);
    let mut pc_ic_map: PCICMap = BTreeMap::new();

    let mut i = 0;
    let mut cumulative_push_size = 0;
    while i < code.len() {
        let op = code[i];
        pc_ic_map.insert(i, i - cumulative_push_size);
        if opcode_infos[op as usize].is_push() {
            // Skip the push bytes.
            //
            // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
            i += (op - opcode::PUSH1 + 1) as usize;
            cumulative_push_size += (op - opcode::PUSH1 + 1) as usize;
        }
        i += 1;
    }

    pc_ic_map
}

/// A map of instruction counters to program counters.
pub type ICPCMap = BTreeMap<usize, usize>;

/// Builds a mapping from instruction counters to program counters.
pub fn build_ic_pc_map(spec: SpecId, code: &[u8]) -> ICPCMap {
    let opcode_infos = spec_opcode_gas(spec);
    let mut ic_pc_map: ICPCMap = ICPCMap::new();

    let mut i = 0;
    let mut cumulative_push_size = 0;
    while i < code.len() {
        let op = code[i];
        ic_pc_map.insert(i - cumulative_push_size, i);
        if opcode_infos[op as usize].is_push() {
            // Skip the push bytes.
            //
            // For more context on the math, see: https://github.com/bluealloy/revm/blob/007b8807b5ad7705d3cacce4d92b89d880a83301/crates/revm/src/interpreter/contract.rs#L114-L115
            i += (op - opcode::PUSH1 + 1) as usize;
            cumulative_push_size += (op - opcode::PUSH1 + 1) as usize;
        }
        i += 1;
    }

    ic_pc_map
}

/// Given an ABI and selector, it tries to find the respective function.
pub fn get_function(
    contract_name: &str,
    selector: &FixedBytes,
    abi: &Abi,
) -> eyre::Result<Function> {
    abi.functions()
        .find(|func| func.short_signature().as_slice() == selector.as_slice())
        .cloned()
        .wrap_err(format!("{contract_name} does not have the selector {selector:?}"))
}
