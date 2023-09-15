use ethers::types::transaction::eip2930::{AccessList, AccessListItem};
use hashbrown::{HashMap, HashSet};
use revm::{
    interpreter::{opcode, InstructionResult, Interpreter},
    primitives::{Address, B256},
    Database, EVMData, Inspector,
};

use crate::utils::{b160_to_h160, b256_to_h256, h160_to_b160, h256_to_b256};

/// An inspector that collects touched accounts and storage slots.
#[derive(Default, Debug)]
pub struct AccessListTracer {
    excluded: HashSet<Address>,
    access_list: HashMap<Address, HashSet<B256>>,
}

impl AccessListTracer {
    pub fn new(
        access_list: AccessList,
        from: Address,
        to: Address,
        precompiles: Vec<Address>,
    ) -> Self {
        AccessListTracer {
            excluded: [from, to].iter().chain(precompiles.iter()).copied().collect(),
            access_list: access_list
                .0
                .iter()
                .map(|v| {
                    (
                        h160_to_b160(v.address),
                        v.storage_keys.iter().copied().map(h256_to_b256).collect(),
                    )
                })
                .collect(),
        }
    }
    pub fn access_list(&self) -> AccessList {
        AccessList::from(
            self.access_list
                .iter()
                .map(|(address, slots)| AccessListItem {
                    address: b160_to_h160(*address),
                    storage_keys: slots.iter().copied().map(b256_to_h256).collect(),
                })
                .collect::<Vec<AccessListItem>>(),
        )
    }
}
impl<DB: Database> Inspector<DB> for AccessListTracer {
    #[inline]
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _data: &mut EVMData<'_, DB>,
    ) -> InstructionResult {
        match interpreter.current_opcode() {
            opcode::SLOAD | opcode::SSTORE => {
                if let Ok(slot) = interpreter.stack().peek(0) {
                    let cur_contract = interpreter.contract.address;
                    self.access_list.entry(cur_contract).or_default().insert(slot.into());
                }
            }
            opcode::EXTCODECOPY |
            opcode::EXTCODEHASH |
            opcode::EXTCODESIZE |
            opcode::BALANCE |
            opcode::SELFDESTRUCT => {
                if let Ok(slot) = interpreter.stack().peek(0) {
                    let addr: Address = Address::from_word(slot.into());
                    if !self.excluded.contains(&addr) {
                        self.access_list.entry(addr).or_default();
                    }
                }
            }
            opcode::DELEGATECALL | opcode::CALL | opcode::STATICCALL | opcode::CALLCODE => {
                if let Ok(slot) = interpreter.stack().peek(1) {
                    let addr: Address = Address::from_word(slot.into());
                    if !self.excluded.contains(&addr) {
                        self.access_list.entry(addr).or_default();
                    }
                }
            }
            _ => (),
        }
        InstructionResult::Continue
    }
}
