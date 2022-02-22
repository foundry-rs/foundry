use super::ExecutorState;
use crate::executor::{
    patch_hardhat_console_selector, HardhatConsoleCalls, HARDHAT_CONSOLE_ADDRESS,
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, RawLog, Token},
    types::{Address, H256, U256},
};
use revm::{
    db::Database, opcode, CallContext, CreateScheme, EVMData, Gas, Inspector, Machine, Return,
    Transfer,
};
use std::cell::RefCell;
use std::rc::Rc;

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the LOG opcodes as well as Hardhat-style logs.
pub struct LogCollector {
    state: Rc<RefCell<ExecutorState>>,
}

impl LogCollector {
    pub fn new(state: Rc<RefCell<ExecutorState>>) -> Self {
        Self { state }
    }

    fn log(&mut self, machine: &Machine, n: u8) {
        let (offset, len) =
            (try_or_return!(machine.stack().peek(0)), try_or_return!(machine.stack().peek(1)));
        let data = if len.is_zero() {
            Vec::new()
        } else {
            machine.memory.get_slice(as_usize_or_return!(offset), as_usize_or_return!(len)).to_vec()
        };

        let n = n as usize;
        let mut topics = Vec::with_capacity(n);
        for i in 0..n {
            let mut topic = H256::zero();
            try_or_return!(machine.stack.peek(2 + i)).to_big_endian(topic.as_bytes_mut());
            topics.push(topic);
        }

        self.state.borrow_mut().logs.push(RawLog { topics, data });
    }

    fn hardhat_log(&mut self, input: Vec<u8>) -> (Return, Bytes) {
        // Patch the Hardhat-style selectors
        let input = patch_hardhat_console_selector(input.to_vec());
        let decoded = match HardhatConsoleCalls::decode(&input) {
            Ok(inner) => inner,
            Err(err) => {
                return (
                    Return::Revert,
                    ethers::abi::encode(&[Token::String(err.to_string())]).into(),
                )
            }
        };

        // Convert it to a DS-style `emit log(string)` event
        self.state.borrow_mut().logs.push(RawLog {
            topics: vec![H256::from_slice(
                &hex::decode("41304facd9323d75b11bcdd609cb38effffdb05710f7caf0e9b16c6d9d709f50")
                    .unwrap(),
            )],
            data: ethers::abi::encode(&[Token::String(decoded.to_string())]),
        });

        (Return::Continue, Bytes::new())
    }
}

impl<DB> Inspector<DB> for LogCollector
where
    DB: Database,
{
    fn initialize(&mut self, _: &mut EVMData<'_, DB>) {}

    fn initialize_machine(&mut self, _: &mut Machine, _: &mut EVMData<'_, DB>, _: bool) -> Return {
        Return::Continue
    }

    fn step(&mut self, machine: &mut Machine, _: &mut EVMData<'_, DB>, _is_static: bool) -> Return {
        match machine.contract.code[machine.program_counter()] {
            opcode::LOG0 => self.log(&machine, 0),
            opcode::LOG1 => self.log(&machine, 1),
            opcode::LOG2 => self.log(&machine, 2),
            opcode::LOG3 => self.log(&machine, 3),
            opcode::LOG4 => self.log(&machine, 4),
            _ => (),
        }

        Return::Continue
    }

    fn step_end(&mut self, _: Return, _: &mut Machine) -> Return {
        Return::Continue
    }

    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        to: Address,
        _: &CallContext,
        _: &Transfer,
        input: &Bytes,
        _: u64,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if to == *HARDHAT_CONSOLE_ADDRESS {
            let (status, reason) = self.hardhat_log(input.to_vec());
            (status, Gas::new(0), reason)
        } else {
            (Return::Continue, Gas::new(0), Bytes::new())
        }
    }

    fn call_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: Address,
        _: &CallContext,
        _: &Transfer,
        _: &Bytes,
        _: u64,
        _: u64,
        _: Return,
        _: &Bytes,
        _: bool,
    ) {
    }

    fn create(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: Address,
        _: &CreateScheme,
        _: U256,
        _: &Bytes,
        _: u64,
    ) -> (Return, Option<Address>, Gas, Bytes) {
        (Return::Continue, None, Gas::new(0), Bytes::new())
    }

    fn create_end(
        &mut self,
        _: &mut EVMData<'_, DB>,
        _: Address,
        _: &CreateScheme,
        _: U256,
        _: &Bytes,
        _: Return,
        _: Option<Address>,
        _: u64,
        _: u64,
        _: &Bytes,
    ) {
    }

    fn selfdestruct(&mut self) {}
}
