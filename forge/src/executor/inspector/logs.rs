use crate::executor::{
    patch_hardhat_console_selector, HardhatConsoleCalls, HARDHAT_CONSOLE_ADDRESS,
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, RawLog, Token},
    types::H256,
};
use revm::{db::Database, opcode, CallInputs, EVMData, Gas, Inspector, Interpreter, Return};

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the LOG opcodes as well as Hardhat-style logs.
pub struct LogCollector {
    pub logs: Vec<RawLog>,
}

impl LogCollector {
    pub fn new() -> Self {
        Self { logs: Vec::new() }
    }

    fn log(&mut self, interpreter: &Interpreter, n: u8) {
        let (offset, len) = (
            try_or_return!(interpreter.stack().peek(0)),
            try_or_return!(interpreter.stack().peek(1)),
        );
        let data = if len.is_zero() {
            Vec::new()
        } else {
            interpreter
                .memory
                .get_slice(as_usize_or_return!(offset), as_usize_or_return!(len))
                .to_vec()
        };

        let n = n as usize;
        let mut topics = Vec::with_capacity(n);
        for i in 0..n {
            let mut topic = H256::zero();
            try_or_return!(interpreter.stack.peek(2 + i)).to_big_endian(topic.as_bytes_mut());
            topics.push(topic);
        }

        self.logs.push(RawLog { topics, data });
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
        self.logs.push(convert_hh_log_to_event(decoded));

        (Return::Continue, Bytes::new())
    }
}

impl<DB> Inspector<DB> for LogCollector
where
    DB: Database,
{
    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        _: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        match interpreter.contract.code[interpreter.program_counter()] {
            opcode::LOG0 => self.log(interpreter, 0),
            opcode::LOG1 => self.log(interpreter, 1),
            opcode::LOG2 => self.log(interpreter, 2),
            opcode::LOG3 => self.log(interpreter, 3),
            opcode::LOG4 => self.log(interpreter, 4),
            _ => (),
        }

        Return::Continue
    }

    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        call: &CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == *HARDHAT_CONSOLE_ADDRESS {
            let (status, reason) = self.hardhat_log(call.input.to_vec());
            (status, Gas::new(0), reason)
        } else {
            (Return::Continue, Gas::new(0), Bytes::new())
        }
    }
}

/// Converts a call to Hardhat's `console.log` to a DSTest `log(string)` event.
fn convert_hh_log_to_event(call: HardhatConsoleCalls) -> RawLog {
    RawLog {
        // This is topic 0 of DSTest's `log(string)`
        topics: vec![H256::from_slice(
            &hex::decode("41304facd9323d75b11bcdd609cb38effffdb05710f7caf0e9b16c6d9d709f50")
                .unwrap(),
        )],
        // Convert the parameters of the call to their string representation for the log
        data: ethers::abi::encode(&[Token::String(call.to_string())]),
    }
}
