use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_sol_types::SolValue;
use revm::interpreter::{opcode, Interpreter};
use std::collections::HashMap;

/// Recorded mapping slots.
#[derive(Clone, Debug, Default)]
pub struct MappingSlots {
    /// Holds mapping parent (slots => slots)
    pub parent_slots: HashMap<B256, B256>,

    /// Holds mapping key (slots => key)
    pub keys: HashMap<B256, B256>,

    /// Holds mapping child (slots => slots[])
    pub children: HashMap<B256, Vec<B256>>,

    /// Holds the last sha3 result `sha3_result => (data_low, data_high)`, this would only record
    /// when sha3 is called with `size == 0x40`, and the lower 256 bits would be stored in
    /// `data_low`, higher 256 bits in `data_high`.
    /// This is needed for mapping_key detect if the slot is for some mapping and record that.
    pub seen_sha3: HashMap<B256, (B256, B256)>,
}

impl MappingSlots {
    /// Tries to insert a mapping slot. Returns true if it was inserted.
    pub fn insert(&mut self, slot: B256) -> bool {
        match self.seen_sha3.get(&slot).copied() {
            Some((key, parent)) => {
                if self.keys.contains_key(&slot) {
                    return false
                }
                self.keys.insert(slot, key);
                self.parent_slots.insert(slot, parent);
                self.children.entry(parent).or_default().push(slot);
                self.insert(parent);
                true
            }
            None => false,
        }
    }
}

impl Cheatcode for startMappingRecordingCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        if state.mapping_slots.is_none() {
            state.mapping_slots = Some(Default::default());
        }
        Ok(Default::default())
    }
}

impl Cheatcode for stopMappingRecordingCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.mapping_slots = None;
        Ok(Default::default())
    }
}

impl Cheatcode for getMappingLengthCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { target, mappingSlot } = self;
        let result = slot_child(state, target, mappingSlot).map(Vec::len).unwrap_or(0);
        Ok((result as u64).abi_encode())
    }
}

impl Cheatcode for getMappingSlotAtCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { target, mappingSlot, idx } = self;
        let result = slot_child(state, target, mappingSlot)
            .and_then(|set| set.get(idx.saturating_to::<usize>()))
            .copied()
            .unwrap_or_default();
        Ok(result.abi_encode())
    }
}

impl Cheatcode for getMappingKeyAndParentOfCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { target, elementSlot: slot } = self;
        let mut found = false;
        let mut key = &B256::ZERO;
        let mut parent = &B256::ZERO;
        if let Some(slots) = mapping_slot(state, target) {
            if let Some(key2) = slots.keys.get(slot) {
                found = true;
                key = key2;
                parent = &slots.parent_slots[slot];
            } else if let Some((key2, parent2)) = slots.seen_sha3.get(slot) {
                found = true;
                key = key2;
                parent = parent2;
            }
        }
        Ok((found, key, parent).abi_encode_params())
    }
}

fn mapping_slot<'a>(state: &'a Cheatcodes, target: &'a Address) -> Option<&'a MappingSlots> {
    state.mapping_slots.as_ref()?.get(target)
}

fn slot_child<'a>(
    state: &'a Cheatcodes,
    target: &'a Address,
    slot: &'a B256,
) -> Option<&'a Vec<B256>> {
    mapping_slot(state, target)?.children.get(slot)
}

#[cold]
pub(crate) fn step(mapping_slots: &mut HashMap<Address, MappingSlots>, interpreter: &Interpreter) {
    match interpreter.current_opcode() {
        opcode::KECCAK256 => {
            if interpreter.stack.peek(1) == Ok(U256::from(0x40)) {
                let address = interpreter.contract.target_address;
                let offset = interpreter.stack.peek(0).expect("stack size > 1").saturating_to();
                let data = interpreter.shared_memory.slice(offset, 0x40);
                let low = B256::from_slice(&data[..0x20]);
                let high = B256::from_slice(&data[0x20..]);
                let result = keccak256(data);

                mapping_slots.entry(address).or_default().seen_sha3.insert(result, (low, high));
            }
        }
        opcode::SSTORE => {
            if let Some(mapping_slots) = mapping_slots.get_mut(&interpreter.contract.target_address)
            {
                if let Ok(slot) = interpreter.stack.peek(0) {
                    mapping_slots.insert(slot.into());
                }
            }
        }
        _ => {}
    }
}
