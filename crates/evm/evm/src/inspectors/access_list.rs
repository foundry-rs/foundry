use alloy_primitives::{Address, B256};
use alloy_rpc_types::{AccessList, AccessListItem};
use hashbrown::{HashMap, HashSet};
use revm::{
    interpreter::{opcode, Interpreter},
    Database, EVMData, Inspector,
};

/// An inspector that collects touched accounts and storage slots.
#[derive(Debug, Default)]
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
                .map(|v| (v.address, v.storage_keys.iter().copied().map(|k| k.into()).collect()))
                .collect(),
        }
    }

    pub fn access_list(&self) -> AccessList {
        AccessList(
            self.access_list
                .iter()
                .map(|(address, slots)| AccessListItem {
                    address: *address,
                    storage_keys: slots.iter().copied().map(|k| k.into()).collect(),
                })
                .collect::<Vec<AccessListItem>>(),
        )
    }
}

impl<DB: Database> Inspector<DB> for AccessListTracer {
    #[inline]
    fn step(&mut self, interpreter: &mut Interpreter<'_>, _data: &mut EVMData<'_, DB>) {
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
    }
}
