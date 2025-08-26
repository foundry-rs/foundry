use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_primitives::{Address, B256, U256, keccak256, map::AddressHashMap};
use alloy_sol_types::SolValue;
use foundry_common::mapping_slots::MappingSlots;
use revm::{
    bytecode::opcode,
    interpreter::{Interpreter, interpreter_types::Jumps},
};

impl Cheatcode for startMappingRecordingCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self {} = self;
        state.mapping_slots.get_or_insert_default();
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
pub(crate) fn step(mapping_slots: &mut AddressHashMap<MappingSlots>, interpreter: &Interpreter) {
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
