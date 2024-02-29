use alloy_primitives::{Address, Bytes, Log, B256};
use alloy_sol_types::{SolEvent, SolInterface, SolValue};
use foundry_common::{fmt::ConsoleFmt, ErrorExt};
use foundry_evm_core::{
    abi::{patch_hh_console_selector, Console, HardhatConsole},
    constants::HARDHAT_CONSOLE_ADDRESS,
};
use revm::{
    interpreter::{CallInputs, Gas, InstructionResult},
    Database, EVMData, Inspector,
};

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the `LOG` opcodes as well as Hardhat-style logs.
#[derive(Clone, Debug, Default)]
pub struct LogCollector {
    /// The collected logs. Includes both `LOG` opcodes and Hardhat-style logs.
    pub logs: Vec<Log>,
}

impl LogCollector {
    fn hardhat_log(&mut self, mut input: Vec<u8>) -> (InstructionResult, Bytes) {
        // Patch the Hardhat-style selector (`uint` instead of `uint256`)
        patch_hh_console_selector(&mut input);

        // Decode the call
        let decoded = match HardhatConsole::HardhatConsoleCalls::abi_decode(&input, false) {
            Ok(inner) => inner,
            Err(err) => return (InstructionResult::Revert, err.abi_encode_revert()),
        };

        // Convert the decoded call to a DS `log(string)` event
        self.logs.push(convert_hh_log_to_event(decoded));

        (InstructionResult::Continue, Bytes::new())
    }
}

impl<DB: Database> Inspector<DB> for LogCollector {
    fn log(&mut self, _: &mut EVMData<'_, DB>, address: &Address, topics: &[B256], data: &Bytes) {
        if let Some(log) = Log::new(*address, topics.to_vec(), data.clone()) {
            self.logs.push(log);
        }
    }

    fn call(
        &mut self,
        _: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
    ) -> (InstructionResult, Gas, Bytes) {
        let (status, reason) = if call.contract == HARDHAT_CONSOLE_ADDRESS {
            self.hardhat_log(call.input.to_vec())
        } else {
            (InstructionResult::Continue, Bytes::new())
        };
        (status, Gas::new(call.gas_limit), reason)
    }
}

/// Converts a call to Hardhat's `console.log` to a DSTest `log(string)` event.
fn convert_hh_log_to_event(call: HardhatConsole::HardhatConsoleCalls) -> Log {
    // Convert the parameters of the call to their string representation using `ConsoleFmt`.
    let fmt = call.fmt(Default::default());
    Log::new(Address::default(), vec![Console::log::SIGNATURE_HASH], fmt.abi_encode().into())
        .unwrap_or_else(|| Log { ..Default::default() })
}
