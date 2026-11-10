//! Registration of constant-folded mapping slots as concrete Keccak preimages.
//!
//! With the optimizer on, solc folds the slot of a mapping entry at a constant key,
//! `keccak256(abi.encode(key, slot))`, into one `PUSH32`. The engine treats a constant slot as a
//! hash only when it knows a preimage, so the folded slot would otherwise count as a fixed slot
//! that a write through a symbolic key never reaches. Before a run, the mapping roots in the
//! storage layout of each deployed contract are hashed with every candidate key: the source
//! literals and the contract's `PUSH` immediates. The nested level of a nested mapping is a
//! product over the keys, so it is searched only when the run accesses a large constant slot of
//! that contract, and only the match is recorded. A registered preimage is a real one, so this
//! adds no unsoundness; constants that no candidate produces, such as ERC-1967 and ERC-7201 roots,
//! stay fixed slots.

use super::*;
use foundry_common::slot_identifier::{ENCODING_INPLACE, ENCODING_MAPPING};
use foundry_compilers::artifacts::{Storage, StorageLayout, StorageType};

/// Candidate keys and storage layouts of the deployed contracts, kept to resolve the nested level
/// of a folded mapping slot on demand.
#[derive(Default)]
pub(crate) struct FoldedSlots {
    contracts: HashMap<Address, (Arc<StorageLayout>, Vec<U256>)>,
    /// Constant slots already searched for a nested preimage.
    searched: HashSet<U256>,
}

impl FoldedSlots {
    /// Records the first-level mapping slots of every deployed contract with a known storage
    /// layout and keeps the layouts and candidate keys for the nested level.
    pub(crate) fn register(
        cx: &mut SymCx,
        executor: &ExecutorView<'_>,
        deadline: Option<Instant>,
    ) -> Self {
        let mut folded = Self::default();
        let Some(known_contracts) = executor.known_contracts else { return folded };
        let literals = executor
            .literals
            .map(|literals| {
                literals
                    .get()
                    .words
                    .values()
                    .flatten()
                    .map(|word| (*word).into())
                    .collect::<HashSet<U256>>()
            })
            .unwrap_or_default();
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
                    if let Ok(slot) = entry.slot.parse::<U256>()
                        && !walk_mapping_slots(
                            cx,
                            layout,
                            slot,
                            &entry.storage_type,
                            &keys,
                            1,
                            &mut |cx, key, slot| {
                                (!past_deadline(deadline))
                                    .then(|| record_mapping_preimage(cx, key, slot))
                            },
                        )
                    {
                        return folded;
                    }
                }
                folded.contracts.insert(address, (layout.clone(), keys));
            }
        }
        folded
    }

    /// Records the preimage of `slot` when it is a folded entry of a nested mapping of the
    /// contract at `address`, `keccak(inner . keccak(outer . root))` plus a struct offset below
    /// `2^64`. Each constant slot is searched once, so a run that never accesses such a slot pays
    /// nothing for the key product.
    pub(crate) fn resolve_nested(
        &mut self,
        cx: &mut SymCx,
        address: Address,
        slot: U256,
        deadline: Option<Instant>,
    ) {
        let max_offset = U256::ONE << STORAGE_LAYOUT_OFFSET_BITS;
        if slot < max_offset || !self.searched.insert(slot) {
            return;
        }
        let Some((layout, keys)) = self.contracts.get(&address) else { return };
        for entry in &layout.storage {
            if let Ok(root) = entry.slot.parse::<U256>()
                && !walk_mapping_slots(
                    cx,
                    layout,
                    root,
                    &entry.storage_type,
                    keys,
                    2,
                    &mut |cx, key, base| {
                        if past_deadline(deadline) {
                            return None;
                        }
                        let (hash, bytes) = mapping_preimage(key, base);
                        if slot.wrapping_sub(hash) < max_offset {
                            record_preimage_bytes(cx, hash, &bytes);
                        }
                        Some(hash)
                    },
                )
            {
                return;
            }
        }
    }
}

fn past_deadline(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|deadline| Instant::now() >= deadline)
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

/// Calls `hash` for every candidate key that fits the key type of the mapping at `slot`, descending
/// into struct members and `levels - 1` nested mapping levels through the returned hash. Returns
/// `false` when `hash` stopped the walk.
fn walk_mapping_slots(
    cx: &mut SymCx,
    layout: &StorageLayout,
    slot: U256,
    type_id: &str,
    keys: &[U256],
    levels: usize,
    hash: &mut impl FnMut(&mut SymCx, U256, U256) -> Option<U256>,
) -> bool {
    let Some(ty) = layout.types.get(type_id) else { return true };
    if levels == 0 {
        return true;
    }
    if ty.encoding == ENCODING_MAPPING {
        let Some((key_ty, value_id)) =
            ty.key.as_deref().and_then(|key| layout.types.get(key)).zip(ty.value.as_deref())
        else {
            return true;
        };
        for &key in keys.iter().filter(|&&key| key_fits(key_ty, key)) {
            let Some(value_slot) = hash(cx, key, slot) else { return false };
            if !walk_mapping_slots(cx, layout, value_slot, value_id, keys, levels - 1, hash) {
                return false;
            }
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
                if !walk_mapping_slots(cx, layout, slot, &member.storage_type, keys, levels, hash) {
                    return false;
                }
            }
        }
    }
    true
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

/// Returns `keccak(key . slot)` and its 64-byte preimage.
fn mapping_preimage(key: U256, slot: U256) -> (U256, [u8; 64]) {
    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(B256::from(key).as_slice());
    bytes[32..].copy_from_slice(B256::from(slot).as_slice());
    (keccak256(bytes).into(), bytes)
}

/// Records `keccak(key . slot)` with its preimage and returns the hash.
fn record_mapping_preimage(cx: &mut SymCx, key: U256, slot: U256) -> U256 {
    let (hash, bytes) = mapping_preimage(key, slot);
    record_preimage_bytes(cx, hash, &bytes);
    hash
}

fn record_preimage_bytes(cx: &mut SymCx, hash: U256, bytes: &[u8; 64]) {
    let preimage = bytes.iter().map(|&byte| SymExpr::constant(cx, U256::from(byte))).collect();
    cx.record_concrete_keccak_preimage(hash, preimage);
}
