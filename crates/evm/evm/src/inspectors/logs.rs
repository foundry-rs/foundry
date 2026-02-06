use alloy_primitives::Log;
use alloy_sol_types::{SolEvent, SolInterface, SolValue};
use foundry_common::{ErrorExt, fmt::ConsoleFmt, sh_println};
use foundry_evm_core::{
    InspectorExt, abi::console, constants::HARDHAT_CONSOLE_ADDRESS, decode::decode_console_log,
};
use revm::{
    Inspector,
    context::ContextTr,
    interpreter::{
        CallInputs, CallOutcome, Gas, InstructionResult, InterpreterResult,
        interpreter::EthInterpreter,
    },
};

/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the `LOG` opcodes as well as Hardhat-style `console.sol` logs.
#[derive(Clone, Debug)]
pub enum LogCollector {
    /// The collected logs. Includes both `LOG` opcodes and Hardhat-style `console.sol` logs.
    Capture { logs: Vec<Log> },
    /// Print logs directly to stdout.
    LiveLogs,
}

impl LogCollector {
    pub fn into_captured_logs(self) -> Option<Vec<Log>> {
        match self {
            Self::Capture { logs } => Some(logs),
            Self::LiveLogs => None,
        }
    }

    #[cold]
    fn do_hardhat_log<CTX>(&mut self, context: &mut CTX, inputs: &CallInputs) -> Option<CallOutcome>
    where
        CTX: ContextTr,
    {
        if let Err(err) = self.hardhat_log(&inputs.input.bytes(context)) {
            let result = InstructionResult::Revert;
            let output = err.abi_encode_revert();
            return Some(CallOutcome {
                result: InterpreterResult { result, output, gas: Gas::new(inputs.gas_limit) },
                memory_offset: inputs.return_memory_offset.clone(),
                was_precompile_called: true,
                precompile_call_logs: vec![],
            });
        }
        None
    }

    fn hardhat_log(&mut self, data: &[u8]) -> alloy_sol_types::Result<()> {
        let decoded = console::hh::ConsoleCalls::abi_decode(data)?;
        self.push_msg(&decoded.fmt(Default::default()));
        Ok(())
    }

    fn push_raw_log(&mut self, log: Log) {
        match self {
            Self::Capture { logs } => logs.push(log),
            Self::LiveLogs => {
                if let Some(msg) = decode_console_log(&log) {
                    sh_println!("{msg}").expect("fail printing to stdout");
                } else {
                    // This case should not happen if the users call through forge-std.
                    // We print the log data for the user nonetheless.
                    sh_println!("console.log({:?}, {})", log.data.topics(), log.data.data)
                        .expect("fail printing to stdout");
                }
            }
        }
    }

    fn push_msg(&mut self, msg: &str) {
        match self {
            Self::Capture { logs } => logs.push(new_console_log(msg)),
            Self::LiveLogs => sh_println!("{msg}").expect("fail printing to stdout"),
        }
    }
}

impl<CTX> Inspector<CTX, EthInterpreter> for LogCollector
where
    CTX: ContextTr,
{
    fn log(&mut self, _context: &mut CTX, log: Log) {
        self.push_raw_log(log);
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
        self.push_msg(msg);
    }
}

/// Creates a `console.log(string)` event.
fn new_console_log(msg: &str) -> Log {
    Log::new_unchecked(
        HARDHAT_CONSOLE_ADDRESS,
        vec![console::ds::log::SIGNATURE_HASH],
        msg.abi_encode().into(),
    )
}
