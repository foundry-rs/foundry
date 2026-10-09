//! `console.log` output.
//!
//! Anvil decodes `console.log` calls in an inspector and prints the lines of every mined
//! transaction. Reth's payload builder has no inspector hook, so a precompile at Hardhat's
//! console address decodes the calls into a per-EVM buffer, and the block executor prints the
//! buffer once the transaction is in a block. The precompile address is warm at the start of a
//! transaction, so the first `console.log` of a transaction costs 2,500 gas less than on anvil.

use crate::logging::LoggingState;
use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use alloy_primitives::{B256, Bytes};
use alloy_sol_types::SolInterface;
use foundry_common::{ErrorExt, fmt::ConsoleFmt};
use foundry_evm_core::abi::console::hh::ConsoleCalls;
use parking_lot::Mutex;
use revm::precompile::{PrecompileId, PrecompileOutput, PrecompileResult};
use std::{borrow::Cow, collections::HashSet, sync::Arc};

pub use foundry_evm_core::constants::HARDHAT_CONSOLE_ADDRESS;

/// The precompile id.
static PRECOMPILE_ID: PrecompileId = PrecompileId::Custom(Cow::Borrowed("console.log"));

/// How many printed transactions the printer remembers before it forgets them all. The engine
/// executes a transaction right after the payload builder, so the set never needs to be large.
const PRINTED_CAPACITY: usize = 4096;

/// Decodes a `console.log` call into its lines.
pub fn decode(data: &[u8]) -> alloy_sol_types::Result<Vec<String>> {
    let call = ConsoleCalls::abi_decode(data)?;
    Ok(call.fmt(Default::default()).lines().map(str::to_owned).collect())
}

/// The `console.log` lines of the transaction an EVM executes, collected by the precompile.
#[derive(Clone, Debug, Default)]
pub struct ConsoleBuffer(Arc<Mutex<Vec<String>>>);

impl ConsoleBuffer {
    /// Returns the precompile that decodes `console.log` calls into this buffer.
    pub fn precompile(&self) -> DynPrecompile {
        let buffer = self.clone();
        DynPrecompile::new_stateful(PRECOMPILE_ID.clone(), move |input: PrecompileInput<'_>| {
            buffer.log(input)
        })
    }

    /// Takes the collected lines.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock())
    }

    /// Drops the collected lines.
    pub fn clear(&self) {
        self.0.lock().clear();
    }

    fn log(&self, input: PrecompileInput<'_>) -> PrecompileResult {
        match decode(input.data) {
            Ok(lines) => {
                self.0.lock().extend(lines);
                Ok(PrecompileOutput::new(0, Bytes::new(), input.reservoir))
            }
            // A call the console library cannot have made reverts, as on anvil.
            Err(error) => {
                Ok(PrecompileOutput::revert(0, error.abi_encode_revert(), input.reservoir))
            }
        }
    }
}

/// Prints the `console.log` lines of mined transactions once, under the node's logging switch.
#[derive(Clone, Debug)]
pub struct ConsolePrinter {
    logging: LoggingState,
    /// The transactions whose lines were printed. The payload builder and the engine both
    /// execute a mined transaction, and reth replays blocks for traces.
    printed: Arc<Mutex<HashSet<B256>>>,
}

impl ConsolePrinter {
    /// Creates the printer.
    pub fn new(logging: LoggingState) -> Self {
        Self { logging, printed: Arc::default() }
    }

    /// Prints the lines of `tx`, unless they were printed before. Returns whether it printed.
    pub fn print(&self, tx: B256, lines: Vec<String>) -> bool {
        if lines.is_empty() {
            return false;
        }
        {
            let mut printed = self.printed.lock();
            if printed.len() >= PRINTED_CAPACITY {
                printed.clear();
            }
            if !printed.insert(tx) {
                return false;
            }
        }
        if self.logging.is_enabled() {
            for line in lines {
                let _ = foundry_common::sh_println!("{line}");
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{U256, keccak256};
    use alloy_sol_types::SolValue;

    fn selector(signature: &str) -> Vec<u8> {
        keccak256(signature)[..4].to_vec()
    }

    #[test]
    fn decodes_console_calls() {
        let mut data = selector("log(string)");
        data.extend(("hello".to_string(),).abi_encode_params());
        assert_eq!(decode(&data).unwrap(), vec!["hello".to_string()]);
        let mut data = selector("log(string,uint256)");
        data.extend(("x is".to_string(), U256::from(7)).abi_encode_params());
        assert_eq!(decode(&data).unwrap(), vec!["x is 7".to_string()]);
        assert!(decode(&[0xde, 0xad, 0xbe, 0xef]).is_err());
    }

    #[test]
    fn prints_each_transaction_once() {
        let printer = ConsolePrinter::new(LoggingState::new(false));
        let tx = B256::repeat_byte(1);
        assert!(!printer.print(tx, Vec::new()));
        assert!(printer.print(tx, vec!["a".to_string()]));
        assert!(!printer.print(tx, vec!["a".to_string()]));
        assert!(printer.print(B256::repeat_byte(2), vec!["b".to_string()]));
    }
}
