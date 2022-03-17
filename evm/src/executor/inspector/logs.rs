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
#[derive(Default)]
pub struct LogCollector {
    pub logs: Vec<RawLog>,
}

impl LogCollector {
    pub fn new() -> Self {
        Default::default()
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
        if let Some(log) = extract_log(interpreter) {
            self.logs.push(log);
        }

        Return::Continue
    }

    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == *HARDHAT_CONSOLE_ADDRESS {
            let (status, reason) = self.hardhat_log(call.input.to_vec());
            (status, Gas::new(call.gas_limit), reason)
        } else {
            (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
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

/// Extracts a log from the interpreter if there is any.
pub fn extract_log(interpreter: &Interpreter) -> Option<RawLog> {
    let num_topics = match interpreter.contract.code[interpreter.program_counter()] {
        opcode::LOG0 => 0,
        opcode::LOG1 => 1,
        opcode::LOG2 => 2,
        opcode::LOG3 => 3,
        opcode::LOG4 => 4,
        _ => return None,
    };

    let (offset, len) = (
        as_usize_or_return!(interpreter.stack().peek(0).ok()?, None),
        as_usize_or_return!(interpreter.stack().peek(1).ok()?, None),
    );
    let data = if len == 0 {
        Vec::new()
    } else {
        // If we're trying to access more memory than exists, we will pretend like that memory is
        // zeroed. We could resize the memory here, but it would mess up the gas accounting REVM
        // does for memory resizes.
        if offset > interpreter.memory.len() {
            vec![0; len]
        } else if offset + len > interpreter.memory.len() {
            let mut data =
                Vec::from(interpreter.memory.get_slice(offset, interpreter.memory.len()));
            data.resize(offset + len, 0);
            data
        } else {
            interpreter.memory.get_slice(offset, len).to_vec()
        }
    };

    let mut topics = Vec::with_capacity(num_topics);
    for i in 0..num_topics {
        let mut topic = H256::zero();
        interpreter.stack.peek(2 + i).ok()?.to_big_endian(topic.as_bytes_mut());
        topics.push(topic);
    }

    Some(RawLog { topics, data })
}
