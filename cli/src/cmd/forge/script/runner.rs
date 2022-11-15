use super::*;
use ethers::types::{Address, Bytes, NameOrAddress, U256};
use forge::{
    executor::{CallResult, DeployResult, EvmError, Executor, RawCallResult},
    revm::{return_ok, Return},
    trace::{TraceKind, Traces},
    CALLER,
};
use tracing::log::trace;

/// Represents which simulation stage is the script execution at.
pub enum SimulationStage {
    Local,
    OnChain,
}

/// Drives script execution
#[derive(Debug)]
pub struct ScriptRunner {
    pub executor: Executor,
    pub initial_balance: U256,
    pub sender: Address,
}

impl ScriptRunner {
    pub fn new(executor: Executor, initial_balance: U256, sender: Address) -> Self {
        Self { executor, initial_balance, sender }
    }

    /// Deploys the libraries and broadcast contract. Calls setUp method if requested.
    pub fn setup(
        &mut self,
        libraries: &[Bytes],
        code: Bytes,
        setup: bool,
        sender_nonce: U256,
        is_broadcast: bool,
        need_create2_deployer: bool,
    ) -> eyre::Result<(Address, ScriptResult)> {
        trace!(target: "script", "executing setUP()");

        if !is_broadcast {
            if self.sender == Config::DEFAULT_SENDER {
                // We max out their balance so that they can deploy and make calls.
                self.executor.set_balance(self.sender, U256::MAX)?;
            }

            if need_create2_deployer {
                self.executor.deploy_create2_deployer()?;
            }
        }

        self.executor.set_nonce(self.sender, sender_nonce.as_u64())?;

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(CALLER, U256::MAX)?;

        // Deploy libraries
        let mut traces: Traces = libraries
            .iter()
            .filter_map(|code| {
                let DeployResult { traces, .. } = self
                    .executor
                    .deploy(self.sender, code.0.clone(), 0u32.into(), None)
                    .expect("couldn't deploy library");

                traces
            })
            .map(|traces| (TraceKind::Deployment, traces))
            .collect();

        // Deploy an instance of the contract
        let DeployResult {
            address,
            mut logs,
            traces: constructor_traces,
            debug: constructor_debug,
            ..
        } = self
            .executor
            .deploy(CALLER, code.0, 0u32.into(), None)
            .map_err(|err| eyre::eyre!("Failed to deploy script:\n{}", err))?;

        traces.extend(constructor_traces.map(|traces| (TraceKind::Deployment, traces)).into_iter());
        self.executor.set_balance(address, self.initial_balance)?;

        // Optionally call the `setUp` function
        let (success, gas_used, labeled_addresses, transactions, debug, script_wallets) = if !setup
        {
            self.executor.backend_mut().set_test_contract(address);
            (
                true,
                0,
                Default::default(),
                None,
                vec![constructor_debug].into_iter().collect(),
                vec![],
            )
        } else {
            match self.executor.setup(Some(self.sender), address) {
                Ok(CallResult {
                    reverted,
                    traces: setup_traces,
                    labels,
                    logs: setup_logs,
                    debug,
                    gas_used,
                    transactions,
                    script_wallets,
                    ..
                }) |
                Err(EvmError::Execution {
                    reverted,
                    traces: setup_traces,
                    labels,
                    logs: setup_logs,
                    debug,
                    gas_used,
                    transactions,
                    script_wallets,
                    ..
                }) => {
                    traces
                        .extend(setup_traces.map(|traces| (TraceKind::Setup, traces)).into_iter());
                    logs.extend_from_slice(&setup_logs);

                    // We call the `setUp()` function with self.sender, and if there haven't been
                    // any broadcasts, then the EVM cheatcode module hasn't corrected the nonce.
                    // So we have to
                    if transactions.is_none() || transactions.as_ref().unwrap().is_empty() {
                        self.executor.set_nonce(
                            self.sender,
                            sender_nonce.as_u64() + libraries.len() as u64,
                        )?;
                    }

                    (
                        !reverted,
                        gas_used,
                        labels,
                        transactions,
                        vec![constructor_debug, debug].into_iter().collect(),
                        script_wallets,
                    )
                }
                Err(e) => return Err(e.into()),
            }
        };

        Ok((
            address,
            ScriptResult {
                returned: bytes::Bytes::new(),
                success,
                gas_used,
                labeled_addresses,
                transactions,
                logs,
                traces,
                debug,
                address: None,
                script_wallets,
            },
        ))
    }

    /// Executes the method that will collect all broadcastable transactions.
    pub fn script(&mut self, address: Address, calldata: Bytes) -> eyre::Result<ScriptResult> {
        self.call(self.sender, address, calldata, U256::zero(), false)
    }

    /// Runs a broadcastable transaction locally and persists its state.
    pub fn simulate(
        &mut self,
        from: Address,
        to: Option<NameOrAddress>,
        calldata: Option<Bytes>,
        value: Option<U256>,
    ) -> eyre::Result<ScriptResult> {
        if let Some(NameOrAddress::Address(to)) = to {
            self.call(from, to, calldata.unwrap_or_default(), value.unwrap_or(U256::zero()), true)
        } else if to.is_none() {
            let (address, gas_used, logs, traces, debug) = match self.executor.deploy(
                from,
                calldata.expect("No data for create transaction").0,
                value.unwrap_or(U256::zero()),
                None,
            ) {
                Ok(DeployResult { address, gas_used, logs, traces, debug, .. }) => {
                    (address, gas_used, logs, traces, debug)
                }
                Err(EvmError::Execution { reason, traces, gas_used, logs, debug, .. }) => {
                    println!("{}", Paint::red(format!("\nFailed with `{reason}`:\n")));

                    (Address::zero(), gas_used, logs, traces, debug)
                }
                e => eyre::bail!("Unrecoverable error: {:?}", e),
            };

            Ok(ScriptResult {
                returned: bytes::Bytes::new(),
                success: address != Address::zero(),
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
                labeled_addresses: Default::default(),
                transactions: Default::default(),
                address: Some(address),
                script_wallets: vec![],
            })
        } else {
            eyre::bail!("ENS not supported.");
        }
    }

    /// Executes the call
    ///
    /// This will commit the changes if `commit` is true.
    ///
    /// This will return _estimated_ gas instead of the precise gas the call would consume, so it
    /// can be used as `gas_limit`.
    fn call(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        commit: bool,
    ) -> eyre::Result<ScriptResult> {
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
            ..
        } = res;

        Ok(ScriptResult {
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
        })
    }
}
