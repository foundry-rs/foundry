use super::*;
use alloy_primitives::{Address, Bytes, U256};
use ethers::types::NameOrAddress;
use eyre::Result;
use forge::{
    executor::{CallResult, DeployResult, EvmError, ExecutionErr, Executor, RawCallResult},
    revm::interpreter::{return_ok, InstructionResult},
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
        sender_nonce: u64,
        is_broadcast: bool,
        need_create2_deployer: bool,
    ) -> Result<(Address, ScriptResult)> {
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

        self.executor.set_nonce(self.sender, sender_nonce)?;

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(CALLER, U256::MAX)?;

        // Deploy libraries
        let mut traces: Traces = libraries
            .iter()
            .filter_map(|code| {
                let DeployResult { traces, .. } = self
                    .executor
                    .deploy(self.sender, code.clone(), U256::ZERO, None)
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
            .deploy(CALLER, code.0.into(), U256::ZERO, None)
            .map_err(|err| eyre::eyre!("Failed to deploy script:\n{}", err))?;

        traces.extend(constructor_traces.map(|traces| (TraceKind::Deployment, traces)));
        self.executor.set_balance(address, self.initial_balance)?;

        // Optionally call the `setUp` function
        let (success, gas_used, labeled_addresses, transactions, debug, script_wallets) = if !setup
        {
            self.executor.backend.set_test_contract(address);
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
                }) => {
                    traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)));
                    logs.extend_from_slice(&setup_logs);

                    self.maybe_correct_nonce(sender_nonce, libraries.len())?;

                    (
                        !reverted,
                        gas_used,
                        labels,
                        transactions,
                        vec![constructor_debug, debug].into_iter().collect(),
                        script_wallets,
                    )
                }
                Err(EvmError::Execution(err)) => {
                    let ExecutionErr {
                        reverted,
                        traces: setup_traces,
                        labels,
                        logs: setup_logs,
                        debug,
                        gas_used,
                        transactions,
                        script_wallets,
                        ..
                    } = *err;
                    traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)));
                    logs.extend_from_slice(&setup_logs);

                    self.maybe_correct_nonce(sender_nonce, libraries.len())?;

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
                returned: Bytes::new(),
                success,
                gas_used,
                labeled_addresses: labeled_addresses
                    .into_iter()
                    .map(|l| (l.0, l.1))
                    .collect::<BTreeMap<_, _>>(),
                transactions,
                logs,
                traces,
                debug,
                address: None,
                script_wallets,
                ..Default::default()
            },
        ))
    }

    /// We call the `setUp()` function with self.sender, and if there haven't been
    /// any broadcasts, then the EVM cheatcode module hasn't corrected the nonce.
    /// So we have to.
    fn maybe_correct_nonce(
        &mut self,
        sender_initial_nonce: u64,
        libraries_len: usize,
    ) -> Result<()> {
        if let Some(cheatcodes) = &self.executor.inspector.cheatcodes {
            if !cheatcodes.corrected_nonce {
                self.executor
                    .set_nonce(self.sender, sender_initial_nonce + libraries_len as u64)?;
            }
            self.executor.inspector.cheatcodes.as_mut().unwrap().corrected_nonce = false;
        }
        Ok(())
    }

    /// Executes the method that will collect all broadcastable transactions.
    pub fn script(&mut self, address: Address, calldata: Bytes) -> Result<ScriptResult> {
        self.call(self.sender, address, calldata, U256::ZERO, false)
    }

    /// Runs a broadcastable transaction locally and persists its state.
    pub fn simulate(
        &mut self,
        from: Address,
        to: Option<NameOrAddress>,
        calldata: Option<Bytes>,
        value: Option<U256>,
    ) -> Result<ScriptResult> {
        if let Some(NameOrAddress::Address(to)) = to {
            self.call(
                from,
                to.to_alloy(),
                calldata.unwrap_or_default(),
                value.unwrap_or(U256::ZERO),
                true,
            )
        } else if to.is_none() {
            let (address, gas_used, logs, traces, debug) = match self.executor.deploy(
                from,
                calldata.expect("No data for create transaction").0.into(),
                value.unwrap_or(U256::ZERO),
                None,
            ) {
                Ok(DeployResult { address, gas_used, logs, traces, debug, .. }) => {
                    (address, gas_used, logs, traces, debug)
                }
                Err(EvmError::Execution(err)) => {
                    let ExecutionErr { reason, traces, gas_used, logs, debug, .. } = *err;
                    println!("{}", Paint::red(format!("\nFailed with `{reason}`:\n")));

                    (Address::ZERO, gas_used, logs, traces, debug)
                }
                Err(e) => eyre::bail!("Failed deploying contract: {e:?}"),
            };

            Ok(ScriptResult {
                returned: Bytes::new(),
                success: address != Address::ZERO,
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
                address: Some(address),
                ..Default::default()
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
    ) -> Result<ScriptResult> {
        let mut res = self.executor.call_raw(from, to, calldata.clone(), value)?;
        let mut gas_used = res.gas_used;

        // We should only need to calculate realistic gas costs when preparing to broadcast
        // something. This happens during the onchain simulation stage, where we commit each
        // collected transactions.
        //
        // Otherwise don't re-execute, or some usecases might be broken: https://github.com/foundry-rs/foundry/issues/3921
        if commit {
            gas_used = self.search_optimal_gas_usage(&res, from, to, &calldata, value)?;
            res = self.executor.call_raw_committing(from, to, calldata, value)?;
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
        let breakpoints = res.cheatcodes.map(|cheats| cheats.breakpoints).unwrap_or_default();

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
            breakpoints,
        })
    }

    /// The executor will return the _exact_ gas value this transaction consumed, setting this value
    /// as gas limit will result in `OutOfGas` so to come up with a better estimate we search over a
    /// possible range we pick a higher gas limit 3x of a succeeded call should be safe.
    ///
    /// This might result in executing the same script multiple times. Depending on the user's goal,
    /// it might be problematic when using `ffi`.
    fn search_optimal_gas_usage(
        &mut self,
        res: &RawCallResult,
        from: Address,
        to: Address,
        calldata: &Bytes,
        value: U256,
    ) -> Result<u64> {
        let mut gas_used = res.gas_used;
        if matches!(res.exit_reason, return_ok!()) {
            // store the current gas limit and reset it later
            let init_gas_limit = self.executor.env.tx.gas_limit;

            let mut highest_gas_limit = gas_used * 3;
            let mut lowest_gas_limit = gas_used;
            let mut last_highest_gas_limit = highest_gas_limit;
            while (highest_gas_limit - lowest_gas_limit) > 1 {
                let mid_gas_limit = (highest_gas_limit + lowest_gas_limit) / 2;
                self.executor.env.tx.gas_limit = mid_gas_limit;
                let res = self.executor.call_raw(from, to, calldata.0.clone().into(), value)?;
                match res.exit_reason {
                    InstructionResult::Revert |
                    InstructionResult::OutOfGas |
                    InstructionResult::OutOfFund => {
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
            self.executor.env.tx.gas_limit = init_gas_limit;
        }
        Ok(gas_used)
    }
}
