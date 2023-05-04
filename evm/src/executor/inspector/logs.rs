use crate::{
    executor::{patch_hardhat_console_selector, HardhatConsoleCalls, HARDHAT_CONSOLE_ADDRESS},
    utils::{b160_to_h160, b256_to_h256, h160_to_b160},
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, Token},
    types::{Log, H256, U256},
};
use foundry_macros::ConsoleFmt;
use revm::{
    interpreter::{CallInputs, Gas, InstructionResult, Interpreter},
    primitives::{B160, B256},
    Database, EVMData, Inspector,
};
use std::fmt::Write;
use yansi::Paint;

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the LOG opcodes as well as Hardhat-style logs.
#[derive(Debug, Clone, Default)]
pub struct LogCollector {
    pub logs: Vec<Log>,

    /// Current's call memory
    /// used for `console.logMemory`
    pub memory: Vec<u8>,
}

impl LogCollector {
    fn hardhat_log(&mut self, mut input: Vec<u8>) -> (InstructionResult, Bytes) {
        // Patch the Hardhat-style selectors
        patch_hardhat_console_selector(&mut input);
        let decoded = match HardhatConsoleCalls::decode(input) {
            Ok(inner) => inner,
            Err(err) => {
                return (
                    InstructionResult::Revert,
                    ethers::abi::encode(&[Token::String(err.to_string())]).into(),
                )
            }
        };

        if let HardhatConsoleCalls::LogMemory(inner) = &decoded {
            println!("inner {inner:?}");
            match format_log_memory(&self.memory, inner.start, inner.end, inner.pretty_print) {
                Ok(formatted_memory) => {
                    let token = Token::String(formatted_memory);
                    let data = ethers::abi::encode(&[token]).into();
                    self.logs.push(Log { topics: vec![TOPIC], data, ..Default::default() });
                }
                Err(err) => {
                    return (
                        InstructionResult::Revert,
                        ethers::abi::encode(&[Token::String(format!("Error logMemory: {err}"))])
                            .into(),
                    )
                }
            };
        } else {
            // Convert it to a DS-style `emit log(string)` event
            self.logs.push(convert_hh_log_to_event(decoded));
        }

        (InstructionResult::Continue, Bytes::new())
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
        _: bool,
    ) -> InstructionResult {
        self.memory = interpreter.memory.data().clone();
        InstructionResult::Continue
    }

    fn log(&mut self, _: &mut EVMData<'_, DB>, address: &B160, topics: &[B256], data: &Bytes) {
        self.logs.push(Log {
            address: b160_to_h160(*address),
            topics: topics.iter().copied().map(b256_to_h256).collect(),
            data: data.clone().into(),
            ..Default::default()
        });
    }

    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (InstructionResult, Gas, Bytes) {
        if call.contract == h160_to_b160(HARDHAT_CONSOLE_ADDRESS) {
            let (status, reason) = self.hardhat_log(call.input.to_vec());
            (status, Gas::new(call.gas_limit), reason)
        } else {
            (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new())
        }
    }
}

/// Topic 0 of DSTest's `log(string)`.
///
/// `0x41304facd9323d75b11bcdd609cb38effffdb05710f7caf0e9b16c6d9d709f50`
const TOPIC: H256 = H256([
    0x41, 0x30, 0x4f, 0xac, 0xd9, 0x32, 0x3d, 0x75, 0xb1, 0x1b, 0xcd, 0xd6, 0x09, 0xcb, 0x38, 0xef,
    0xff, 0xfd, 0xb0, 0x57, 0x10, 0xf7, 0xca, 0xf0, 0xe9, 0xb1, 0x6c, 0x6d, 0x9d, 0x70, 0x9f, 0x50,
]);

/// Converts a call to Hardhat's `console.log` to a DSTest `log(string)` event.
fn convert_hh_log_to_event(call: HardhatConsoleCalls) -> Log {
    // Convert the parameters of the call to their string representation using `ConsoleFmt`.
    let fmt = call.fmt(Default::default());
    let token = Token::String(fmt);
    let data = ethers::abi::encode(&[token]).into();
    Log { topics: vec![TOPIC], data, ..Default::default() }
}

fn check_log_memory_inputs(
    start: U256,
    end: U256,
    memory_length: u32,
) -> Result<(u32, u32), String> {
    let start = u32::try_from(start).map_err(|err| format!("start parameter: {}", err))?;
    let end = u32::try_from(end).map_err(|err| format!("end parameter: {}", err))?;
    if start > end {
        return Err(format!("invalid parameters: start ({}) must be <= end ({})", start, end))
    }
    if end > memory_length - 1 {
        return Err(format!(
            "invalid parameters: end ({}). Max memory offset: {}",
            end,
            memory_length - 1
        ))
    }
    Ok((start, end))
}

fn format_log_memory(
    mem: &Vec<u8>,
    start: U256,
    end: U256,
    pretty_print: bool,
) -> Result<String, String> {
    let (start, end) = check_log_memory_inputs(start, end, mem.len() as u32)?;

    let memory_start = start - (start % 32);
    let memory_end = end + (31 - end % 32);

    let mem =
        &mem[(memory_start as usize)..=(memory_end as usize)].chunks(32).collect::<Vec<&[u8]>>();

    let mut formatted_mem = vec![];

    for (i_chunk, chunk) in mem.iter().enumerate() {
        let mut s = String::new();

        write!(
            &mut s,
            "[{:#04x}:{:#04x}] ",
            (memory_start as usize) + i_chunk * 32,
            (memory_start as usize) + (i_chunk + 1) * 32
        )
        .map_err(|err| err.to_string())?;

        for (i_value, value) in chunk.iter().enumerate() {
            let i = (i_chunk * 32 + i_value) as u32;
            let requested_range = (start - memory_start)..=(end - memory_start);

            let value = if requested_range.contains(&i) {
                Paint::yellow(value).bold()
            } else {
                Paint::new(value)
            };

            write!(&mut s, "{:02x?}", value).map_err(|err| err.to_string())?;
            if pretty_print {
                write!(&mut s, " ").map_err(|err| err.to_string())?;
            }
        }

        formatted_mem.push(s);
    }

    let header = (0..32).map(|x| format!("{:>2?}", x)).collect::<Vec<String>>().join(" ");
    let formatted_mem = formatted_mem.iter().fold(String::new(), |acc, s| acc + s + "\n");

    let mem_as_string = if pretty_print {
        format!("            {}\n{}", header, formatted_mem)
    } else {
        formatted_mem
    };
    Ok(format!("LogMemory:\n{}", mem_as_string))
}
