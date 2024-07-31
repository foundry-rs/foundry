use alloy_primitives::{Bytes, Log};
use alloy_sol_types::{SolEvent, SolInterface, SolValue};
use foundry_common::{fmt::ConsoleFmt, ErrorExt};
use foundry_evm_core::{
    abi::{patch_hh_console_selector, Console, HardhatConsole},
    constants::HARDHAT_CONSOLE_ADDRESS,
    InspectorExt,
};
use revm::{
    interpreter::{
        CallInputs, CallOutcome, Gas, InstructionResult, Interpreter, InterpreterResult,
    },
    Database, EvmContext, Inspector,
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
    fn log(&mut self, _interp: &mut Interpreter, _context: &mut EvmContext<DB>, log: &Log) {
        self.logs.push(log.clone());
    }

    fn call(
        &mut self,
        _context: &mut EvmContext<DB>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        if inputs.target_address == HARDHAT_CONSOLE_ADDRESS {
            let (res, out) = self.hardhat_log(inputs.input.to_vec());
            if res != InstructionResult::Continue {
                return Some(CallOutcome {
                    result: InterpreterResult {
                        result: res,
                        output: out,
                        gas: Gas::new(inputs.gas_limit),
                    },
                    memory_offset: inputs.return_memory_offset.clone(),
                })
            }
        }

        None
    }
}

impl<DB: Database> InspectorExt<DB> for LogCollector {
    fn console_log(&mut self, input: String) {
        self.logs.push(Log::new_unchecked(
            HARDHAT_CONSOLE_ADDRESS,
            vec![Console::log::SIGNATURE_HASH],
            input.abi_encode().into(),
        ));
    }
}

/// Converts a call to Hardhat's `console.log` to a DSTest `log(string)` event.
fn convert_hh_log_to_event(call: HardhatConsole::HardhatConsoleCalls) -> Log {
    // Convert the parameters of the call to their string representation using `ConsoleFmt`.
    let fmt = call.fmt(Default::default());
    Log::new_unchecked(
        HARDHAT_CONSOLE_ADDRESS,
        vec![Console::log::SIGNATURE_HASH],
        fmt.abi_encode().into(),
    )
}
