//! ChiselRunner
//!
//! This module contains the `ChiselRunner` struct, which assists with deploying
//! and calling the REPL contract on a in-memory REVM instance.

use alloy_primitives::{Address, Bytes, Log, U256};
use eyre::Result;
use foundry_evm::{
    executors::{DeployResult, Executor, RawCallResult},
    traces::{TraceKind, Traces},
};
use revm::interpreter::{return_ok, InstructionResult};
use std::collections::HashMap;

/// The function selector of the REPL contract's entrypoint, the `run()` function.
static RUN_SELECTOR: [u8; 4] = [0xc0, 0x40, 0x62, 0x26];

/// The Chisel Runner
///
/// Based off of foundry's forge cli runner for scripting.
/// See: [runner](cli::cmd::forge::script::runner.rs)
#[derive(Debug)]
pub struct ChiselRunner {
    /// The Executor
    pub executor: Executor,
    /// An initial balance
    pub initial_balance: U256,
    /// The sender
    pub sender: Address,
    /// Input calldata appended to `RUN_SELECTOR`
    pub input: Option<Vec<u8>>,
}

/// Represents the result of a Chisel REPL run
#[derive(Debug, Default)]
pub struct ChiselResult {
    /// Was the run a success?
    pub success: bool,
    /// Transaction logs
    pub logs: Vec<Log>,
    /// Call traces
    pub traces: Traces,
    /// Amount of gas used in the transaction
    pub gas_used: u64,
    /// Map of addresses to their labels
    pub labeled_addresses: HashMap<Address, String>,
    /// Return data
    pub returned: Bytes,
    /// Called address
    pub address: Option<Address>,
    /// EVM State at the final instruction of the `run()` function
    pub state: Option<(Vec<U256>, Vec<u8>, InstructionResult)>,
}

/// ChiselRunner implementation
impl ChiselRunner {
    /// Create a new [ChiselRunner]
    ///
    /// ### Takes
    ///
    /// An [Executor], the initial balance of the sender, and the sender's [Address].
    ///
    /// ### Returns
    ///
    /// A new [ChiselRunner]
    pub fn new(
        executor: Executor,
        initial_balance: U256,
        sender: Address,
        input: Option<Vec<u8>>,
    ) -> Self {
        Self { executor, initial_balance, sender, input }
    }

    /// Run a contract as a REPL session
    ///
    /// ### Takes
    ///
    /// The creation bytecode of the REPL contract
    ///
    /// ### Returns
    ///
    /// Optionally, a tuple containing the deployed address of the bytecode as well as a
    /// [ChiselResult] containing information about the result of the call to the deployed REPL
    /// contract.
    pub fn run(&mut self, bytecode: Bytes) -> Result<(Address, ChiselResult)> {
        // Set the sender's balance to [U256::MAX] for deployment of the REPL contract.
        self.executor.set_balance(self.sender, U256::MAX)?;

        // Deploy an instance of the REPL contract
        // We don't care about deployment traces / logs here
        let DeployResult { address, .. } = self
            .executor
            .deploy(self.sender, bytecode, U256::ZERO, None)
            .map_err(|err| eyre::eyre!("Failed to deploy REPL contract:\n{}", err))?;

        // Reset the sender's balance to the initial balance for calls.
        self.executor.set_balance(self.sender, self.initial_balance)?;

        // Append the input to the `RUN_SELECTOR` to form the calldata
        let mut calldata = RUN_SELECTOR.to_vec();
        if let Some(mut input) = self.input.clone() {
            calldata.append(&mut input);
        }

        // Call the "run()" function of the REPL contract
        let call_res = self.call(self.sender, address, Bytes::from(calldata), U256::from(0), true);

        call_res.map(|res| (address, res))
    }

    /// Executes the call.
    ///
    /// This will commit the changes if `commit` is true.
    ///
    /// This will return _estimated_ gas instead of the precise gas the call would consume, so it
    /// can be used as `gas_limit`.
    ///
    /// Taken from Forge's script runner.
    fn call(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        commit: bool,
    ) -> eyre::Result<ChiselResult> {
        let fs_commit_changed =
            if let Some(cheatcodes) = &mut self.executor.inspector_mut().cheatcodes {
                let original_fs_commit = cheatcodes.fs_commit;
                cheatcodes.fs_commit = false;
                original_fs_commit != cheatcodes.fs_commit
            } else {
                false
            };

        let mut res = self.executor.call_raw(from, to, calldata.clone(), value)?;
        let mut gas_used = res.gas_used;
        if matches!(res.exit_reason, return_ok!()) {
            // store the current gas limit and reset it later
            let init_gas_limit = self.executor.env().tx.gas_limit;

            // the executor will return the _exact_ gas value this transaction consumed, setting
            // this value as gas limit will result in `OutOfGas` so to come up with a
            // better estimate we search over a possible range we pick a higher gas
            // limit 3x of a succeeded call should be safe
            let mut highest_gas_limit = gas_used * 3;
            let mut lowest_gas_limit = gas_used;
            let mut last_highest_gas_limit = highest_gas_limit;
            while (highest_gas_limit - lowest_gas_limit) > 1 {
                let mid_gas_limit = (highest_gas_limit + lowest_gas_limit) / 2;
                self.executor.env_mut().tx.gas_limit = mid_gas_limit;
                let res = self.executor.call_raw(from, to, calldata.clone(), value)?;
                match res.exit_reason {
                    InstructionResult::Revert |
                    InstructionResult::OutOfGas |
                    InstructionResult::OutOfFunds => {
                        lowest_gas_limit = mid_gas_limit;
                    }
                    _ => {
                        highest_gas_limit = mid_gas_limit;
                        // if last two successful estimations only vary by 10%, we consider this to
                        // sufficiently accurate
                        const ACCURACY: u64 = 10;
                        if (last_highest_gas_limit - highest_gas_limit) * ACCURACY /
                            last_highest_gas_limit <
                            1
                        {
                            // update the gas
                            gas_used = highest_gas_limit;
                            break;
                        }
                        last_highest_gas_limit = highest_gas_limit;
                    }
                }
            }
            // reset gas limit in the
            self.executor.env_mut().tx.gas_limit = init_gas_limit;
        }

        // if we changed `fs_commit` during gas limit search, re-execute the call with original
        // value
        if fs_commit_changed {
            if let Some(cheatcodes) = &mut self.executor.inspector_mut().cheatcodes {
                cheatcodes.fs_commit = !cheatcodes.fs_commit;
            }

            res = self.executor.call_raw(from, to, calldata.clone(), value)?;
        }

        if commit {
            // if explicitly requested we can now commit the call
            res = self.executor.transact_raw(from, to, calldata, value)?;
        }

        let RawCallResult { result, reverted, logs, traces, labels, chisel_state, .. } = res;

        Ok(ChiselResult {
            returned: result,
            success: !reverted,
            gas_used,
            logs,
            traces: traces
                .map(|traces| {
                    // Manually adjust gas for the trace to add back the stipend/real used gas

                    vec![(TraceKind::Execution, traces)]
                })
                .unwrap_or_default(),
            labeled_addresses: labels,
            address: None,
            state: chisel_state,
        })
    }
}
