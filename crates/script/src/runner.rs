use super::ScriptResult;
use crate::build::ScriptPredeployLibraries;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_rpc_types::TransactionRequest;
use eyre::Result;
use foundry_cheatcodes::BroadcastableTransaction;
use foundry_config::Config;
use foundry_evm::{
    constants::{CALLER, DEFAULT_CREATE2_DEPLOYER},
    executors::{DeployResult, EvmError, ExecutionErr, Executor, RawCallResult},
    opts::EvmOpts,
    revm::interpreter::{return_ok, InstructionResult},
    traces::{TraceKind, Traces},
};
use std::collections::VecDeque;
use yansi::Paint;

/// Drives script execution
#[derive(Debug)]
pub struct ScriptRunner {
    pub executor: Executor,
    pub evm_opts: EvmOpts,
}

impl ScriptRunner {
    pub fn new(executor: Executor, evm_opts: EvmOpts) -> Self {
        Self { executor, evm_opts }
    }

    /// Deploys the libraries and broadcast contract. Calls setUp method if requested.
    pub fn setup(
        &mut self,
        libraries: &ScriptPredeployLibraries,
        code: Bytes,
        setup: bool,
        sender_nonce: u64,
        is_broadcast: bool,
        need_create2_deployer: bool,
    ) -> Result<(Address, ScriptResult)> {
        trace!(target: "script", "executing setUP()");

        if !is_broadcast {
            if self.evm_opts.sender == Config::DEFAULT_SENDER {
                // We max out their balance so that they can deploy and make calls.
                self.executor.set_balance(self.evm_opts.sender, U256::MAX)?;
            }

            if need_create2_deployer {
                self.executor.deploy_create2_deployer()?;
            }
        }

        self.executor.set_nonce(self.evm_opts.sender, sender_nonce)?;

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(CALLER, U256::MAX)?;

        let mut library_transactions = VecDeque::new();
        let mut traces = Traces::default();

        // Deploy libraries
        match libraries {
            ScriptPredeployLibraries::Default(libraries) => libraries.iter().for_each(|code| {
                let result = self
                    .executor
                    .deploy(self.evm_opts.sender, code.clone(), U256::ZERO, None)
                    .expect("couldn't deploy library")
                    .raw;

                if let Some(deploy_traces) = result.traces {
                    traces.push((TraceKind::Deployment, deploy_traces));
                }

                library_transactions.push_back(BroadcastableTransaction {
                    rpc: self.evm_opts.fork_url.clone(),
                    transaction: TransactionRequest {
                        from: Some(self.evm_opts.sender),
                        input: code.clone().into(),
                        nonce: Some(sender_nonce + library_transactions.len() as u64),
                        ..Default::default()
                    }
                    .into(),
                })
            }),
            ScriptPredeployLibraries::Create2(libraries, salt) => {
                for library in libraries {
                    let address =
                        DEFAULT_CREATE2_DEPLOYER.create2_from_code(salt, library.as_ref());
                    // Skip if already deployed
                    if !self.executor.is_empty_code(address)? {
                        continue;
                    }
                    let calldata = [salt.as_ref(), library.as_ref()].concat();
                    let result = self
                        .executor
                        .transact_raw(
                            self.evm_opts.sender,
                            DEFAULT_CREATE2_DEPLOYER,
                            calldata.clone().into(),
                            U256::from(0),
                        )
                        .expect("couldn't deploy library");

                    if let Some(deploy_traces) = result.traces {
                        traces.push((TraceKind::Deployment, deploy_traces));
                    }

                    library_transactions.push_back(BroadcastableTransaction {
                        rpc: self.evm_opts.fork_url.clone(),
                        transaction: TransactionRequest {
                            from: Some(self.evm_opts.sender),
                            input: calldata.into(),
                            nonce: Some(sender_nonce + library_transactions.len() as u64),
                            to: Some(TxKind::Call(DEFAULT_CREATE2_DEPLOYER)),
                            ..Default::default()
                        }
                        .into(),
                    });
                }

                // Sender nonce is not incremented when performing CALLs. We need to manually
                // increase it.
                self.executor.set_nonce(
                    self.evm_opts.sender,
                    sender_nonce + library_transactions.len() as u64,
                )?;
            }
        };

        let address = CALLER.create(self.executor.get_nonce(CALLER)?);

        // Set the contracts initial balance before deployment, so it is available during the
        // construction
        self.executor.set_balance(address, self.evm_opts.initial_balance)?;

        // Deploy an instance of the contract
        let DeployResult {
            address,
            raw: RawCallResult { mut logs, traces: constructor_traces, .. },
        } = self
            .executor
            .deploy(CALLER, code, U256::ZERO, None)
            .map_err(|err| eyre::eyre!("Failed to deploy script:\n{}", err))?;

        traces.extend(constructor_traces.map(|traces| (TraceKind::Deployment, traces)));

        // Optionally call the `setUp` function
        let (success, gas_used, labeled_addresses, transactions) = if !setup {
            self.executor.backend_mut().set_test_contract(address);
            (true, 0, Default::default(), Some(library_transactions))
        } else {
            match self.executor.setup(Some(self.evm_opts.sender), address, None) {
                Ok(RawCallResult {
                    reverted,
                    traces: setup_traces,
                    labels,
                    logs: setup_logs,
                    gas_used,
                    transactions: setup_transactions,
                    ..
                }) => {
                    traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)));
                    logs.extend_from_slice(&setup_logs);

                    if let Some(txs) = setup_transactions {
                        library_transactions.extend(txs);
                    }

                    (!reverted, gas_used, labels, Some(library_transactions))
                }
                Err(EvmError::Execution(err)) => {
                    let RawCallResult {
                        reverted,
                        traces: setup_traces,
                        labels,
                        logs: setup_logs,
                        gas_used,
                        transactions,
                        ..
                    } = err.raw;
                    traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)));
                    logs.extend_from_slice(&setup_logs);

                    if let Some(txs) = transactions {
                        library_transactions.extend(txs);
                    }

                    (!reverted, gas_used, labels, Some(library_transactions))
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
                labeled_addresses,
                transactions,
                logs,
                traces,
                address: None,
                ..Default::default()
            },
        ))
    }

    /// Executes the method that will collect all broadcastable transactions.
    pub fn script(&mut self, address: Address, calldata: Bytes) -> Result<ScriptResult> {
        self.call(self.evm_opts.sender, address, calldata, U256::ZERO, false)
    }

    /// Runs a broadcastable transaction locally and persists its state.
    pub fn simulate(
        &mut self,
        from: Address,
        to: Option<Address>,
        calldata: Option<Bytes>,
        value: Option<U256>,
    ) -> Result<ScriptResult> {
        if let Some(to) = to {
            self.call(from, to, calldata.unwrap_or_default(), value.unwrap_or(U256::ZERO), true)
        } else if to.is_none() {
            let res = self.executor.deploy(
                from,
                calldata.expect("No data for create transaction"),
                value.unwrap_or(U256::ZERO),
                None,
            );
            let (address, RawCallResult { gas_used, logs, traces, .. }) = match res {
                Ok(DeployResult { address, raw }) => (address, raw),
                Err(EvmError::Execution(err)) => {
                    let ExecutionErr { raw, reason } = *err;
                    println!("{}", format!("\nFailed with `{reason}`:\n").red());
                    (Address::ZERO, raw)
                }
                Err(e) => eyre::bail!("Failed deploying contract: {e:?}"),
            };

            Ok(ScriptResult {
                returned: Bytes::new(),
                success: address != Address::ZERO,
                gas_used,
                logs,
                // Manually adjust gas for the trace to add back the stipend/real used gas
                traces: traces
                    .map(|traces| vec![(TraceKind::Execution, traces)])
                    .unwrap_or_default(),
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
            res = self.executor.transact_raw(from, to, calldata, value)?;
        }

        let RawCallResult { result, reverted, logs, traces, labels, transactions, .. } = res;
        let breakpoints = res.cheatcodes.map(|cheats| cheats.breakpoints).unwrap_or_default();

        Ok(ScriptResult {
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
            transactions,
            address: None,
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
            // Store the current gas limit and reset it later.
            let init_gas_limit = self.executor.env().tx.gas_limit;

            let mut highest_gas_limit = gas_used * 3;
            let mut lowest_gas_limit = gas_used;
            let mut last_highest_gas_limit = highest_gas_limit;
            while (highest_gas_limit - lowest_gas_limit) > 1 {
                let mid_gas_limit = (highest_gas_limit + lowest_gas_limit) / 2;
                self.executor.env_mut().tx.gas_limit = mid_gas_limit;
                let res = self.executor.call_raw(from, to, calldata.0.clone().into(), value)?;
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
            // Reset gas limit in the executor.
            self.executor.env_mut().tx.gas_limit = init_gas_limit;
        }
        Ok(gas_used)
    }
}
