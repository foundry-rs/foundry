use ethers::{
    prelude::{types::U256, Address},
    signers::LocalWallet,
    types::{transaction::eip2718::TypedTransaction, Bytes, Log},
};
use forge::{
    debug::DebugArena,
    executor::{DeployResult, Executor, RawCallResult},
    trace::{CallTraceArena, TraceKind},
};
use revm::{return_ok, Return};
use std::collections::{BTreeMap, VecDeque};

/// The Chisel Runner
///
/// Based off of foundry's forge cli runner for scripting.
/// See: [runner](cli::cmd::forge::script::runner.rs)
#[derive(Debug)]
pub struct ChiselRunner {
    /// The Executor
    pub executor: Executor,
    /// An initial balance
    /// TODO: can default this to U256::MAX probs.
    pub initial_balance: U256,
    /// The sender
    pub sender: Address,
}

/// Represents the result of a Chisel REPL run
#[derive(Debug, Default)]
#[allow(missing_docs)] // TODO
pub struct ChiselResult {
    /// Was the run a success?
    pub success: bool,
    pub logs: Vec<Log>,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: Option<Vec<DebugArena>>,
    pub gas_used: u64,
    pub labeled_addresses: BTreeMap<Address, String>,
    pub transactions: Option<VecDeque<TypedTransaction>>,
    pub returned: bytes::Bytes,
    pub address: Option<Address>,
    pub script_wallets: Vec<LocalWallet>,
    pub state: Option<(revm::Stack, revm::Memory, revm::Return)>,
}

/// ChiselRunner implementation
impl ChiselRunner {
    /// Create a new [ChiselRunner]
    pub fn new(executor: Executor, initial_balance: U256, sender: Address) -> Self {
        Self { executor, initial_balance, sender }
    }

    /// Run a contract as a REPL session
    pub fn run(&mut self, bytecode: Bytes) -> eyre::Result<(Address, ChiselResult)> {
        // Set the sender's balance to [U256::MAX] for deployment of the REPL contract.
        self.executor.set_balance(self.sender, U256::MAX)?;

        // Deploy an instance of the REPL contract
        // We don't care about deployment traces / logs here
        let DeployResult { address, .. } = self
            .executor
            .deploy(self.sender, bytecode.0, 0.into(), None)
            .map_err(|err| eyre::eyre!("Failed to deploy REPL contract:\n{}", err))?;

        // Reset the sender's balance to the initial balance for calls.
        self.executor.set_balance(self.sender, self.initial_balance)?;

        // Call the "run()" function of the REPL contract
        let call_res =
            self.call(self.sender, address, Bytes::from([0xc0, 0x40, 0x62, 0x26]), 0.into(), true);

        match call_res {
            Ok(call_res) => Ok((address, call_res)),
            Err(e) => Err(e),
        }
    }

    /// Executes the call
    ///
    /// This will commit the changes if `commit` is true.
    ///
    /// This will return _estimated_ gas instead of the precise gas the call would consume, so it
    /// can be used as `gas_limit`.
    ///
    /// Taken directly from Forge's Script Runner
    fn call(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        commit: bool,
    ) -> eyre::Result<ChiselResult> {
        let fs_commit_changed =
            if let Some(ref mut cheatcodes) = self.executor.inspector_config_mut().cheatcodes {
                let original_fs_commit = cheatcodes.fs_commit;
                cheatcodes.fs_commit = false;
                original_fs_commit != cheatcodes.fs_commit
            } else {
                false
            };

        let mut res = self.executor.call_raw(from, to, calldata.0.clone(), value)?;
        let mut gas_used = res.gas_used;
        if matches!(res.exit_reason, return_ok!()) {
            // store the current gas limit and reset it later
            let init_gas_limit = self.executor.env_mut().tx.gas_limit;

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
                let res = self.executor.call_raw(from, to, calldata.0.clone(), value)?;
                match res.exit_reason {
                    Return::Revert |
                    Return::OutOfGas |
                    Return::LackOfFundForGasLimit |
                    Return::OutOfFund => {
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
                            break
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
            if let Some(ref mut cheatcodes) = self.executor.inspector_config_mut().cheatcodes {
                cheatcodes.fs_commit = !cheatcodes.fs_commit;
            }

            res = self.executor.call_raw(from, to, calldata.0.clone(), value)?;
        }

        if commit {
            // if explicitly requested we can now commit the call
            res = self.executor.call_raw_committing(from, to, calldata.0, value)?;
        }

        let RawCallResult {
            result,
            reverted,
            logs,
            traces,
            labels,
            debug,
            transactions,
            script_wallets,
            chisel_state,
            ..
        } = res;

        Ok(ChiselResult {
            returned: result,
            success: !reverted,
            gas_used,
            logs,
            traces: traces
                .map(|mut traces| {
                    // Manually adjust gas for the trace to add back the stipend/real used gas
                    traces.arena[0].trace.gas_cost = gas_used;
                    vec![(TraceKind::Execution, traces)]
                })
                .unwrap_or_default(),
            debug: vec![debug].into_iter().collect(),
            labeled_addresses: labels,
            transactions,
            address: None,
            script_wallets,
            state: chisel_state,
        })
    }
}
