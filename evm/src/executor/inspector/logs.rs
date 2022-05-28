use crate::executor::{
    patch_hardhat_console_selector, HardhatConsoleCalls, HARDHAT_CONSOLE_ADDRESS,
};
use bytes::Bytes;
use ethers::{
    abi::{AbiDecode, Token},
    types::{Address, Log, H256},
};
use revm::{db::Database, CallInputs, EVMData, Gas, Inspector, Return};

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the LOG opcodes as well as Hardhat-style logs.
#[derive(Default)]
pub struct LogCollector {
    pub logs: Vec<Log>,
}

impl LogCollector {
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
    fn log(&mut self, _: &mut EVMData<'_, DB>, address: &Address, topics: &[H256], data: &Bytes) {
        self.logs.push(Log {
            address: *address,
            topics: topics.to_vec(),
            data: data.clone().into(),
            ..Default::default()
        });
    }

    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        _: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == HARDHAT_CONSOLE_ADDRESS {
            let (status, reason) = self.hardhat_log(call.input.to_vec());
            (status, Gas::new(call.gas_limit), reason)
        } else {
            (Return::Continue, Gas::new(call.gas_limit), Bytes::new())
        }
    }
}

/// Converts a call to Hardhat's `console.log` to a DSTest `log(string)` event.
fn convert_hh_log_to_event(call: HardhatConsoleCalls) -> Log {
    Log {
        // This is topic 0 of DSTest's `log(string)`
        topics: vec![H256::from_slice(
            &hex::decode("41304facd9323d75b11bcdd609cb38effffdb05710f7caf0e9b16c6d9d709f50")
                .unwrap(),
        )],
        // Convert the parameters of the call to their string representation for the log
        data: ethers::abi::encode(&[Token::String(call.to_string())]).into(),
        ..Default::default()
    }
}
