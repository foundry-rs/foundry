use alloy_primitives::Log;
use alloy_sol_types::{SolEvent, SolInterface, SolValue};
use evm2::{
    EvmTypesHost, Inspector,
    interpreter::{InstrStop, Interpreter, Message, MessageResult},
};
use foundry_cheatcodes::message_result;
use foundry_common::{ErrorExt, fmt::ConsoleFmt, sh_println};
use foundry_evm_core::{
    abi::console, constants::HARDHAT_CONSOLE_ADDRESS, decode::decode_console_log,
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
    fn do_hardhat_log<T: EvmTypesHost>(
        &mut self,
        message: &Message<T>,
    ) -> Option<MessageResult<T>> {
        if let Err(err) = self.hardhat_log(&message.input) {
            return Some(message_result(message, InstrStop::Revert, err.abi_encode_revert()));
        }
        None
    }

    fn hardhat_log(&mut self, data: &[u8]) -> alloy_sol_types::Result<()> {
        let decoded = console::hh::ConsoleCalls::abi_decode(data)?;
        for line in decoded.fmt(Default::default()).lines() {
            self.console_log(line);
        }
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

    /// Records a `console.log` message, such as one emitted by a cheatcode.
    pub fn console_log(&mut self, msg: &str) {
        match self {
            Self::Capture { logs } => logs.push(new_console_log(msg)),
            Self::LiveLogs => sh_println!("{msg}").expect("fail printing to stdout"),
        }
    }
}

impl<T: EvmTypesHost> Inspector<T> for LogCollector {
    fn log(&mut self, log: &Log, _host: &mut T::Host<'_>) {
        self.push_raw_log(log.clone());
    }

    fn call(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        if message.destination == HARDHAT_CONSOLE_ADDRESS {
            return self.do_hardhat_log::<T>(message);
        }
        None
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
