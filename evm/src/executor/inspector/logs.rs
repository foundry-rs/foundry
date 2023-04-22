use crate::{
    executor::{patch_hardhat_console_selector, HardhatConsoleCalls, HARDHAT_CONSOLE_ADDRESS},
    utils::{b160_to_h160, b256_to_h256, h160_to_b160},
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, Token},
    types::{Log, H256},
};
use foundry_macros::ConsoleFmt;
use revm::{
    interpreter::{CallInputs, Gas, InstructionResult},
    primitives::{B160, B256},
    Database, EVMData, Inspector,
};

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the LOG opcodes as well as Hardhat-style logs.
#[derive(Debug, Clone, Default)]
pub struct LogCollector {
    pub logs: Vec<Log>,
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

        // Convert it to a DS-style `emit log(string)` event
        self.logs.push(convert_hh_log_to_event(decoded));

        (InstructionResult::Continue, Bytes::new())
    }
}

impl<DB> Inspector<DB> for LogCollector
where
    DB: Database,
{
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
