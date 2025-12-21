use alloy_primitives::{
    B256, U256, keccak256,
    map::{AddressHashMap, B256HashMap},
};
use revm::{
    bytecode::opcode,
    interpreter::{Interpreter, interpreter_types::Jumps},
};

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

/// Function to be used in Inspector::step to record mapping slots and keys
#[cold]
pub fn step(mapping_slots: &mut AddressHashMap<MappingSlots>, interpreter: &Interpreter) {
    match interpreter.bytecode.opcode() {
        opcode::KECCAK256 => {
            if interpreter.stack.peek(1) == Ok(U256::from(0x40)) {
                let address = interpreter.input.target_address;
                let offset = interpreter.stack.peek(0).expect("stack size > 1").saturating_to();
                let data = interpreter.memory.slice_len(offset, 0x40);
                let low = B256::from_slice(&data[..0x20]);
                let high = B256::from_slice(&data[0x20..]);
                let result = keccak256(&*data);

                mapping_slots.entry(address).or_default().seen_sha3.insert(result, (low, high));
            }
        }
        opcode::SSTORE => {
            if let Some(mapping_slots) = mapping_slots.get_mut(&interpreter.input.target_address)
                && let Ok(slot) = interpreter.stack.peek(0)
            {
                mapping_slots.insert(slot.into());
            }
        }
        _ => {}
    }
}
