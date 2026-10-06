use alloy_primitives::{
    Address, B256, U256,
    map::{AddressHashMap, B256HashMap},
};
use revm::{
    bytecode::opcode,
    interpreter::{Interpreter, interpreter_types::Jumps},
};

/// Provenance recovered for a Solidity mapping storage slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingProvenance {
    /// The terminal mapping root slot.
    pub root_slot: B256,
    /// Mapping keys ordered from the root mapping to the accessed value.
    pub keys: Vec<B256>,
}

/// Recorded mapping slots.
#[derive(Clone, Debug, Default)]
pub struct MappingSlots {
    /// Holds mapping parent (slots => slots)
    pub parent_slots: B256HashMap<B256>,

    /// Holds mapping key (slots => key)
    pub keys: B256HashMap<B256>,

    /// Holds mapping child (slots => slots[])
    pub children: B256HashMap<Vec<B256>>,

    /// Holds the last sha3 result `sha3_result => (data_low, data_high)`, this would only record
    /// when sha3 is called with `size == 0x40`, and the lower 256 bits would be stored in
    /// `data_low`, higher 256 bits in `data_high`.
    /// This is needed for mapping_key detect if the slot is for some mapping and record that.
    pub seen_sha3: B256HashMap<(B256, B256)>,
}

impl MappingSlots {
    /// Records the result and two input words of a 64-byte `KECCAK256` operation.
    pub fn record_hash(&mut self, result: B256, key: B256, parent: B256) {
        self.seen_sha3.insert(result, (key, parent));
    }

    /// Resolves a computed slot to its terminal mapping root and root-to-leaf keys.
    pub fn resolve(&self, slot: B256) -> Option<MappingProvenance> {
        let mut current = slot;
        let mut keys = Vec::new();
        while let Some((key, parent)) = self.seen_sha3.get(&current).copied() {
            keys.push(key);
            current = parent;
            if keys.len() > self.seen_sha3.len() {
                return None;
            }
        }
        if keys.is_empty() {
            return None;
        }
        keys.reverse();
        Some(MappingProvenance { root_slot: current, keys })
    }

    /// Tries to insert a mapping slot. Returns true if it was inserted.
    pub fn insert(&mut self, slot: B256) -> bool {
        match self.seen_sha3.get(&slot).copied() {
            Some((key, parent)) => {
                if self.keys.insert(slot, key).is_some() {
                    return false;
                }
                self.parent_slots.insert(slot, parent);
                self.children.entry(parent).or_default().push(slot);
                self.insert(parent);
                true
            }
            None => false,
        }
    }
}

/// A pending 64-byte Keccak operation captured before execution.
#[derive(Clone, Copy, Debug)]
pub struct PendingMappingHash {
    /// The effective storage address of the executing frame.
    pub address: Address,
    /// The memory offset containing the Keccak preimage.
    pub offset: usize,
}

/// Captures a 64-byte Keccak operation before execution.
pub fn capture_hash(interpreter: &Interpreter) -> Option<PendingMappingHash> {
    capture_hash_parts(
        interpreter.bytecode.opcode(),
        interpreter.input.target_address,
        interpreter.stack.peek(0).ok(),
        interpreter.stack.peek(1).ok(),
    )
}

/// Captures a 64-byte Keccak operation from the instruction `opcode` about to execute in
/// `address`, given its `offset` and `size` stack operands.
pub fn capture_hash_parts(
    opcode: u8,
    address: Address,
    offset: Option<U256>,
    size: Option<U256>,
) -> Option<PendingMappingHash> {
    if opcode != opcode::KECCAK256 || size? != U256::from(0x40) {
        return None;
    }
    Some(PendingMappingHash { address, offset: offset?.try_into().ok()? })
}

/// Records a successfully executed 64-byte Keccak operation after memory expansion.
pub fn record_hash(
    mapping_slots: &mut AddressHashMap<MappingSlots>,
    interpreter: &Interpreter,
    pending: PendingMappingHash,
) {
    let Ok(result) = interpreter.stack.peek(0) else { return };
    record_hash_parts(
        mapping_slots,
        result,
        interpreter.memory.slice_len(pending.offset, 0x40).as_ref(),
        pending,
    );
}

/// Records a successfully executed 64-byte Keccak operation from its `result` and the 64-byte
/// `preimage` read at `pending.offset`.
pub fn record_hash_parts(
    mapping_slots: &mut AddressHashMap<MappingSlots>,
    result: U256,
    preimage: &[u8],
    pending: PendingMappingHash,
) {
    let key = B256::from_slice(&preimage[..0x20]);
    let parent = B256::from_slice(&preimage[0x20..0x40]);
    mapping_slots.entry(pending.address).or_default().record_hash(result.into(), key, parent);
}

/// Function to be used in `Inspector::step` to record mapping slots.
#[cold]
pub fn step(mapping_slots: &mut AddressHashMap<MappingSlots>, interpreter: &Interpreter) {
    step_parts(
        mapping_slots,
        interpreter.bytecode.opcode(),
        interpreter.input.target_address,
        interpreter.stack.peek(0).ok(),
    );
}

/// Records the mapping slot written by the instruction `opcode` about to execute in `address`,
/// given its `slot` stack operand.
#[cold]
pub fn step_parts(
    mapping_slots: &mut AddressHashMap<MappingSlots>,
    opcode: u8,
    address: Address,
    slot: Option<U256>,
) {
    if opcode == opcode::SSTORE
        && let Some(mapping_slots) = mapping_slots.get_mut(&address)
        && let Some(slot) = slot
    {
        mapping_slots.insert(slot.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::keccak256;

    #[test]
    fn resolves_mapping_keys_from_root_to_leaf() {
        let root = B256::with_last_byte(1);
        let owner = B256::with_last_byte(2);
        let spender = B256::with_last_byte(3);
        let inner = keccak256([owner.as_slice(), root.as_slice()].concat());
        let slot = keccak256([spender.as_slice(), inner.as_slice()].concat());
        let mut slots = MappingSlots::default();
        slots.record_hash(inner, owner, root);
        slots.record_hash(slot, spender, inner);

        assert_eq!(
            slots.resolve(slot),
            Some(MappingProvenance { root_slot: root, keys: vec![owner, spender] })
        );
    }

    #[test]
    fn rejects_plain_slots_offsets_and_cycles() {
        let root = B256::with_last_byte(1);
        let key = B256::with_last_byte(2);
        let slot = keccak256([key.as_slice(), root.as_slice()].concat());
        let mut slots = MappingSlots::default();
        slots.record_hash(slot, key, root);

        assert!(slots.resolve(root).is_none());
        assert!(slots.resolve(B256::from(U256::from_be_bytes(slot.0) + U256::ONE)).is_none());

        slots.record_hash(root, key, slot);
        assert!(slots.resolve(slot).is_none());
    }
}
