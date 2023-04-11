use ethers::{
    abi::{ethereum_types::BigEndianHash, Address},
    types::{
        transaction::eip2930::{AccessList, AccessListItem},
        H256
    },
};
use hashbrown::{HashMap, HashSet};
use revm::{opcode, Database, EVMData, Inspector, Interpreter, Return};

use crate::utils::b160_to_h160;

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
            excluded: vec![from, to].iter().chain(precompiles.iter()).copied().collect(),
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

impl<DB> Inspector<DB> for AccessListTracer
where
    DB: Database,
{
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _data: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        let pc = interpreter.program_counter();
        let op = interpreter.contract.bytecode.bytecode()[pc];

        match op {
            opcode::SLOAD | opcode::SSTORE => {
                if let Ok(slot) = interpreter.stack().peek(0) {
                    let cur_contract = interpreter.contract.address;
                    self.access_list
<<<<<<< HEAD
<<<<<<< HEAD
                        .entry(cur_contract)
=======
                        .entry(H160::from_slice(cur_contract.as_bytes()))
>>>>>>> 30fa1965 (fix(inspector): inspector top level files compile)
=======
                        .entry(b160_to_h160(cur_contract))
>>>>>>> 30d3fe2a (chore: move all manual conversions to use utils)
                        .or_default()
                        .insert(H256::from_uint(&slot));
                }
            }
            opcode::EXTCODECOPY |
            opcode::EXTCODEHASH |
            opcode::EXTCODESIZE |
            opcode::BALANCE |
            opcode::SELFDESTRUCT => {
                if let Ok(slot) = interpreter.stack().peek(0) {
                    let addr: Address = H256::from_uint(&slot).into();
                    if !self.excluded.contains(&addr) {
                        self.access_list.entry(addr).or_default();
                    }
                }
            }
            opcode::DELEGATECALL | opcode::CALL | opcode::STATICCALL | opcode::CALLCODE => {
                if let Ok(slot) = interpreter.stack().peek(1) {
                    let addr: Address = H256::from_uint(&slot).into();
                    if !self.excluded.contains(&addr) {
                        self.access_list.entry(addr).or_default();
                    }
                }
            }
            _ => (),
        }

        Return::Continue
    }
}
