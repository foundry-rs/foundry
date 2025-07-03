use alloy_primitives::Log;
use alloy_sol_types::{SolEvent, SolInterface, SolValue};
use foundry_common::{ErrorExt, fmt::ConsoleFmt};
use foundry_evm_core::{
    InspectorExt, abi::console, backend::DatabaseError, constants::HARDHAT_CONSOLE_ADDRESS,
};
use revm::{
    Database, Inspector,
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{
        CallInputs, CallOutcome, Gas, InstructionResult, Interpreter, InterpreterResult,
        interpreter::EthInterpreter,
    },
};

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the `LOG` opcodes as well as Hardhat-style `console.sol` logs.
#[derive(Clone, Debug, Default)]
pub struct LogCollector {
    /// The collected logs. Includes both `LOG` opcodes and Hardhat-style `console.sol` logs.
    pub logs: Vec<Log>,
}

impl LogCollector {
    #[cold]
    fn do_hardhat_log<CTX>(&mut self, context: &mut CTX, inputs: &CallInputs) -> Option<CallOutcome>
    where
        CTX: ContextTr<Db: Database<Error = DatabaseError>, Journal: JournalExt>,
    {
        if let Err(err) = self.hardhat_log(&inputs.input.bytes(context)) {
            let result = InstructionResult::Revert;
            let output = err.abi_encode_revert();
            return Some(CallOutcome {
                result: InterpreterResult { result, output, gas: Gas::new(inputs.gas_limit) },
                memory_offset: inputs.return_memory_offset.clone(),
            });
        }
        None
    }

    fn hardhat_log(&mut self, data: &[u8]) -> alloy_sol_types::Result<()> {
        let decoded = console::hh::ConsoleCalls::abi_decode(data)?;
        self.logs.push(hh_to_ds(&decoded));
        Ok(())
    }
}

impl<CTX, D> Inspector<CTX, EthInterpreter> for LogCollector
where
    D: Database<Error = DatabaseError>,
    CTX: ContextTr<Db = D>,
    CTX::Journal: JournalExt,
{
    fn log(&mut self, _interp: &mut Interpreter, _context: &mut CTX, log: Log) {
        self.logs.push(log);
    }

    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.target_address == HARDHAT_CONSOLE_ADDRESS {
            return self.do_hardhat_log(context, inputs);
        }
        None
    }
}

impl InspectorExt for LogCollector {
    fn console_log(&mut self, msg: &str) {
        self.logs.push(new_console_log(msg));
    }
}

/// Converts a Hardhat `console.log` call to a DSTest `log(string)` event.
fn hh_to_ds(call: &console::hh::ConsoleCalls) -> Log {
    // Convert the parameters of the call to their string representation using `ConsoleFmt`.
    let msg = call.fmt(Default::default());
    new_console_log(&msg)
}

/// Creates a `console.log(string)` event.
fn new_console_log(msg: &str) -> Log {
    Log::new_unchecked(
        HARDHAT_CONSOLE_ADDRESS,
        vec![console::ds::log::SIGNATURE_HASH],
        msg.abi_encode().into(),
    )
}
