//! Registration of constant-folded mapping slots as concrete Keccak preimages.
//!
//! With the optimizer on, solc folds the slot of a mapping entry at a constant key,
//! `keccak256(abi.encode(key, slot))`, into one `PUSH32`. The engine treats a constant slot as a
//! hash only when it knows a preimage, so the folded slot would otherwise count as a fixed slot
//! that a write through a symbolic key never reaches. Before a run, the mapping roots in the
//! storage layout of each deployed contract are hashed with every candidate key: the source
//! literals and the contract's `PUSH` immediates. Nested mappings get one more level. A registered
//! preimage is a real one, so this adds no unsoundness; constants that no candidate produces, such
//! as ERC-1967 and ERC-7201 roots, stay fixed slots.

use super::*;
use foundry_common::slot_identifier::{ENCODING_INPLACE, ENCODING_MAPPING};
use foundry_compilers::artifacts::{Storage, StorageLayout, StorageType};

/// Mapping levels hashed below a storage layout root: the mapping itself and one nested mapping.
const FOLDED_MAPPING_LEVELS: usize = 2;

/// Records the folded mapping slots of every deployed contract with a known storage layout.
pub(crate) fn register_folded_mapping_slots(cx: &mut SymCx, executor: &ExecutorView<'_>) {
    let Some(known_contracts) = executor.known_contracts else { return };
    let literals = executor
        .literals
        .map(|literals| {
            literals.get().words.values().flatten().map(|word| (*word).into()).collect()
        })
        .unwrap_or_else(HashSet::<U256>::default);
    for &address in &executor.account_addresses {
        if let Ok(Some(account)) = executor.backend.basic(address)
            && let Some(code) = &account.code
            && !code.is_empty()
            && let Some((_, contract)) =
                known_contracts.find_by_deployed_code_exact(code.original_byte_slice())
            && let Some(layout) = &contract.storage_layout
        {
            let mut keys = literals.clone();
            keys.extend(push_immediates(code.original_byte_slice()));
            let keys = keys.into_iter().collect::<Vec<_>>();
            for entry in &layout.storage {
                if let Ok(slot) = entry.slot.parse::<U256>() {
                    register_mapping_slots(
                        cx,
                        layout,
                        slot,
                        &entry.storage_type,
                        &keys,
                        FOLDED_MAPPING_LEVELS,
                    );
                }
            }
        }
    }
}

/// Returns the right-aligned values of the `PUSH` immediates in `code`.
fn push_immediates(code: &[u8]) -> impl Iterator<Item = U256> {
    let mut pc = 0;
    std::iter::from_fn(move || {
        while let Some(&op) = code.get(pc) {
            pc += 1;
            if let opcode::PUSH1..=opcode::PUSH32 = op {
                let len = usize::from(op - opcode::PUSH1 + 1);
                let data = &code[pc..code.len().min(pc + len)];
                pc += len;
                return Some(U256::from_be_slice(data));
            }
        }
        None
    })
}

/// Hashes every candidate key that fits the key type of the mapping at `slot`, descending into
/// struct members and `levels - 1` nested mapping levels.
fn register_mapping_slots(
    cx: &mut SymCx,
    layout: &StorageLayout,
    slot: U256,
    type_id: &str,
    keys: &[U256],
    levels: usize,
) {
    let Some(ty) = layout.types.get(type_id) else { return };
    if levels == 0 {
        return;
    }
    if ty.encoding == ENCODING_MAPPING {
        let Some((key_ty, value_id)) =
            ty.key.as_deref().and_then(|key| layout.types.get(key)).zip(ty.value.as_deref())
        else {
            return;
        };
        for &key in keys.iter().filter(|&&key| key_fits(key_ty, key)) {
            let hash = record_mapping_preimage(cx, key, slot);
            register_mapping_slots(cx, layout, hash, value_id, keys, levels - 1);
        }
    } else if ty.encoding == ENCODING_INPLACE
        && let Some(members) = ty
            .other
            .get("members")
            .and_then(|members| serde_json::from_value::<Vec<Storage>>(members.clone()).ok())
    {
        for member in &members {
            if let Ok(offset) = member.slot.parse::<U256>() {
                let slot = slot.wrapping_add(offset);
                register_mapping_slots(cx, layout, slot, &member.storage_type, keys, levels);
            }
        }
    }
}

/// Returns whether `key` is a value of the mapping key type `ty` as `abi.encode` pads it.
fn key_fits(ty: &StorageType, key: U256) -> bool {
    let Ok(bytes) = ty.number_of_bytes.parse::<usize>() else { return false };
    let bits = bytes * 8;
    if bits == 0 || bits > 256 || ty.label == "string" || ty.label == "bytes" {
        // Dynamic keys are hashed from their bytes, not from a padded word.
        false
    } else if ty.label.starts_with("int") {
        let magnitude = U256::ONE << (bits - 1);
        key < magnitude || key >= U256::ZERO.wrapping_sub(magnitude)
    } else if ty.label.starts_with("bytes") {
        key.trailing_zeros() >= 256 - bits
    } else {
        bits == 256 || key < U256::ONE << bits
    }
}

/// Records `keccak(key . slot)` with its 64-byte preimage and returns the hash.
fn record_mapping_preimage(cx: &mut SymCx, key: U256, slot: U256) -> U256 {
    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(B256::from(key).as_slice());
    bytes[32..].copy_from_slice(B256::from(slot).as_slice());
    let hash = keccak256(bytes).into();
    let preimage = bytes.iter().map(|&byte| SymExpr::constant(cx, U256::from(byte))).collect();
    cx.record_concrete_keccak_preimage(hash, preimage);
    hash
}
