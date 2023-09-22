use super::Cheatcodes;
use alloy_primitives::Bytes;
use ethers::{
    abi::{self, Token},
    types::{Address, U256},
    utils::keccak256,
};
use foundry_utils::types::ToEthers;
use revm::{
    interpreter::{opcode, Interpreter},
    Database, EVMData,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct MappingSlots {
    /// Holds mapping parent (slots => slots)
    pub parent_slots: BTreeMap<U256, U256>,

    /// Holds mapping key (slots => key)
    pub keys: BTreeMap<U256, U256>,

    /// Holds mapping child (slots => slots[])
    pub children: BTreeMap<U256, Vec<U256>>,

    /// Holds the last sha3 result `sha3_result => (data_low, data_high)`, this would only record
    /// when sha3 is called with `size == 0x40`, and the lower 256 bits would be stored in
    /// `data_low`, higher 256 bits in `data_high`.
    /// This is needed for mapping_key detect if the slot is for some mapping and record that.
    pub seen_sha3: BTreeMap<U256, (U256, U256)>,
}

impl MappingSlots {
    pub fn insert(&mut self, slot: U256) -> bool {
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

pub fn get_mapping_length(state: &Cheatcodes, address: Address, slot: U256) -> Bytes {
    let result = match state.mapping_slots.as_ref().and_then(|dict| dict.get(&address)) {
        Some(mapping_slots) => {
            mapping_slots.children.get(&slot).map(|set| set.len()).unwrap_or_default()
        }
        None => 0,
    };
    abi::encode(&[Token::Uint(result.into())]).into()
}

pub fn get_mapping_slot_at(state: &Cheatcodes, address: Address, slot: U256, index: U256) -> Bytes {
    let result = match state.mapping_slots.as_ref().and_then(|dict| dict.get(&address)) {
        Some(mapping_slots) => mapping_slots
            .children
            .get(&slot)
            .and_then(|set| set.get(index.as_usize()))
            .copied()
            .unwrap_or_default(),
        None => 0.into(),
    };
    abi::encode(&[Token::Uint(result)]).into()
}

pub fn get_mapping_key_and_parent(state: &Cheatcodes, address: Address, slot: U256) -> Bytes {
    let (found, key, parent) =
        match state.mapping_slots.as_ref().and_then(|dict| dict.get(&address)) {
            Some(mapping_slots) => match mapping_slots.keys.get(&slot) {
                Some(key) => (true, *key, mapping_slots.parent_slots[&slot]),
                None => match mapping_slots.seen_sha3.get(&slot).copied() {
                    Some(maybe_info) => (true, maybe_info.0, maybe_info.1),
                    None => (false, U256::zero(), U256::zero()),
                },
            },
            None => (false, U256::zero(), U256::zero()),
        };
    abi::encode(&[Token::Bool(found), Token::Uint(key), Token::Uint(parent)]).into()
}

pub fn on_evm_step<DB: Database>(
    mapping_slots: &mut BTreeMap<Address, MappingSlots>,
    interpreter: &Interpreter,
    _data: &mut EVMData<'_, DB>,
) {
    match interpreter.current_opcode() {
        opcode::KECCAK256 => {
            if interpreter.stack.peek(1) == Ok(revm::primitives::U256::from(0x40)) {
                let address = interpreter.contract.address;
                let offset = interpreter.stack.peek(0).expect("stack size > 1").to::<usize>();
                let low = U256::from(interpreter.memory.slice(offset, 0x20));
                let high = U256::from(interpreter.memory.slice(offset + 0x20, 0x20));
                let result = U256::from(keccak256(interpreter.memory.slice(offset, 0x40)));

                mapping_slots
                    .entry(address.to_ethers())
                    .or_default()
                    .seen_sha3
                    .insert(result, (low, high));
            }
        }
        opcode::SSTORE => {
            if let Some(mapping_slots) =
                mapping_slots.get_mut(&interpreter.contract.address.to_ethers())
            {
                if let Ok(slot) = interpreter.stack.peek(0) {
                    mapping_slots.insert(slot.to_ethers());
                }
            }
        }
        _ => {}
    }
}
