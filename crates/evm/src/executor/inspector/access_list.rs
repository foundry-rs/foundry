use ethers::{
    abi::{ethereum_types::BigEndianHash, Address},
    types::{
        transaction::eip2930::{AccessList, AccessListItem},
        H256,
    },
};
use hashbrown::{HashMap, HashSet};
use revm::{
    interpreter::{opcode, InstructionResult, Interpreter},
    Database, EVMData, Inspector,
};

use crate::utils::{b160_to_h160, ru256_to_u256};

/// An inspector that collects touched accounts and storage slots.
#[derive(Default, Debug)]
pub struct AccessListTracer {
    excluded: HashSet<Address>,
    access_list: HashMap<Address, HashSet<H256>>,
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
                .map(|v| (v.address, v.storage_keys.iter().copied().collect()))
                .collect(),
        }
    }

    pub fn access_list(&self) -> AccessList {
        AccessList::from(
            self.access_list
                .iter()
                .map(|(address, slots)| AccessListItem {
                    address: *address,
                    storage_keys: slots.iter().copied().collect(),
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
                    self.access_list
                        .entry(b160_to_h160(cur_contract))
                        .or_default()
                        .insert(H256::from_uint(&ru256_to_u256(slot)));
                }
            }
            opcode::EXTCODECOPY |
            opcode::EXTCODEHASH |
            opcode::EXTCODESIZE |
            opcode::BALANCE |
            opcode::SELFDESTRUCT => {
                if let Ok(slot) = interpreter.stack().peek(0) {
                    let addr: Address = H256::from_uint(&ru256_to_u256(slot)).into();
                    if !self.excluded.contains(&addr) {
                        self.access_list.entry(addr).or_default();
                    }
                }
            }
            opcode::DELEGATECALL | opcode::CALL | opcode::STATICCALL | opcode::CALLCODE => {
                if let Ok(slot) = interpreter.stack().peek(1) {
                    let addr: Address = H256::from_uint(&ru256_to_u256(slot)).into();
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
