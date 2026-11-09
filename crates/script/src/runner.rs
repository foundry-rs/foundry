use super::{ScriptConfig, ScriptResult};
use crate::build::ScriptPredeployLibraries;
use alloy_eips::eip7702::SignedAuthorization;
use alloy_evm::{
    FromRecoveredTx,
    revm::context::{Block, Transaction},
};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, Bytes, TxKind, U256, map::AddressHashMap};
use eyre::Result;
use foundry_cheatcodes::BroadcastableTransaction;
use foundry_common::{LIBRARY_DEPLOYER, TransactionMaybeSigned};
use foundry_config::Config;
use foundry_evm::{
    constants::CALLER,
    core::{
        FoundryBlock, FoundryTransaction,
        evm::{EvmEnvFor, FoundryEvmNetwork, TransactionRequestFor, TxEnvFor},
    },
    executors::{DeployResult, EvmError, ExecutionErr, Executor, RawCallResult},
    opts::EvmOpts,
    revm::interpreter::{InstructionResult, return_ok},
    traces::{TraceKind, Traces},
};
use std::collections::VecDeque;

/// Drives script execution
#[derive(Debug)]
pub struct ScriptRunner<FEN: FoundryEvmNetwork> {
    pub executor: Executor<FEN>,
    pub evm_opts: EvmOpts,
    collect_debug_bytecodes: bool,
}

impl<FEN: FoundryEvmNetwork> ScriptRunner<FEN> {
    pub const fn new(executor: Executor<FEN>, evm_opts: EvmOpts) -> Self {
        Self { executor, evm_opts, collect_debug_bytecodes: false }
    }

    pub const fn with_debug_bytecodes(mut self, collect_debug_bytecodes: bool) -> Self {
        self.collect_debug_bytecodes = collect_debug_bytecodes;
        self
    }

    fn maybe_debug_bytecodes(
        &self,
        debug_bytecodes: AddressHashMap<Bytes>,
    ) -> AddressHashMap<Bytes> {
        if self.collect_debug_bytecodes { debug_bytecodes } else { Default::default() }
    }

    fn extend_debug_bytecodes(
        &self,
        target: &mut AddressHashMap<Bytes>,
        debug_bytecodes: AddressHashMap<Bytes>,
    ) {
        if self.collect_debug_bytecodes {
            target.extend(debug_bytecodes);
        }
    }

    fn deploy_local_libraries(
        &mut self,
        libraries: &[foundry_linking::LinkedLibrary],
        debug_bytecodes: &mut AddressHashMap<Bytes>,
    ) -> Result<()> {
        if libraries.is_empty() {
            return Ok(());
        }
        let balance = self.executor.get_balance(LIBRARY_DEPLOYER)?;
        let nonce = self.executor.get_nonce(LIBRARY_DEPLOYER)?;
        self.executor.set_balance(LIBRARY_DEPLOYER, U256::MAX)?;
        self.executor.set_nonce(LIBRARY_DEPLOYER, 0)?;
        for library in libraries {
            let DeployResult { address, raw } = self
                .executor
                .deploy(LIBRARY_DEPLOYER, library.bytecode.clone(), U256::ZERO, None)
                .map_err(|err| eyre::eyre!("couldn't deploy local library: {err}"))?;
            eyre::ensure!(
                library.address == address,
                "local library deployed at an unexpected address"
            );
            self.extend_debug_bytecodes(debug_bytecodes, raw.debug_bytecodes);
        }
        self.executor.set_balance(LIBRARY_DEPLOYER, balance)?;
        self.executor.set_nonce(LIBRARY_DEPLOYER, nonce)?;
        Ok(())
    }

    /// Deploys the libraries and broadcast contract. Calls setUp method if requested.
    pub fn setup(
        &mut self,
        libraries: &ScriptPredeployLibraries,
        code: Bytes,
        setup: bool,
        script_config: &ScriptConfig<FEN>,
        is_broadcast: bool,
    ) -> Result<(Address, ScriptResult<FEN::Network>)> {
        trace!(target: "script", "executing setUP()");

        if !is_broadcast {
            if self.evm_opts.sender == Config::DEFAULT_SENDER {
                // We max out their balance so that they can deploy and make calls.
                self.executor.set_balance(self.evm_opts.sender, U256::MAX)?;
            }

            if script_config.evm_opts.fork_url.is_none()
                && !script_config.evm_opts.networks.is_tempo()
            {
                self.executor.deploy_create2_deployer()?;
            }
        }

        let sender_nonce = script_config.sender_nonce;
        self.executor.set_nonce(self.evm_opts.sender, sender_nonce)?;

        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(CALLER, U256::MAX)?;

        let mut library_transactions = VecDeque::new();
        let mut traces = Traces::default();
        let mut debug_bytecodes: AddressHashMap<Bytes> = Default::default();

        // Deploy libraries
        match libraries {
            ScriptPredeployLibraries::Default { onchain, local } => {
                self.deploy_local_libraries(local, &mut debug_bytecodes)?;
                for library in onchain {
                    let code = &library.bytecode;
                    let RawCallResult {
                        traces: deploy_traces,
                        debug_bytecodes: deploy_debug_bytecodes,
                        ..
                    } = self
                        .executor
                        .deploy(self.evm_opts.sender, code.clone(), U256::ZERO, None)
                        .map_err(|err| eyre::eyre!("couldn't deploy library: {err}"))?
                        .raw;

                    self.extend_debug_bytecodes(&mut debug_bytecodes, deploy_debug_bytecodes);

                    if let Some(deploy_traces) = deploy_traces {
                        traces.push((TraceKind::Deployment, deploy_traces));
                    }

                    let mut tx_req = TransactionRequestFor::<FEN>::default()
                        .with_from(self.evm_opts.sender)
                        .with_input(code.clone())
                        .with_nonce(sender_nonce + library_transactions.len() as u64);

                    script_config.tempo.apply::<FEN::Network>(&mut tx_req, None);

                    library_transactions.push_back(BroadcastableTransaction {
                        rpc: self.evm_opts.fork_url.clone(),
                        transaction: TransactionMaybeSigned::new(tx_req),
                        contract_address: None,
                    })
                }
            }
            ScriptPredeployLibraries::Create2 { onchain, salt, local } => {
                self.deploy_local_libraries(local, &mut debug_bytecodes)?;
                let create2_deployer = self.executor.create2_deployer();
                for library in onchain {
                    let address =
                        create2_deployer.create2_from_code(salt, library.bytecode.as_ref());
                    // Skip if already deployed
                    if !self.executor.is_empty_code(address)? {
                        continue;
                    }
                    let calldata = [salt.as_ref(), library.bytecode.as_ref()].concat();
                    let RawCallResult {
                        traces: deploy_traces,
                        debug_bytecodes: deploy_debug_bytecodes,
                        ..
                    } = self
                        .executor
                        .transact_raw(
                            self.evm_opts.sender,
                            create2_deployer,
                            calldata.clone().into(),
                            U256::ZERO,
                        )
                        .map_err(|err| eyre::eyre!("couldn't deploy library: {err}"))?;

                    self.extend_debug_bytecodes(&mut debug_bytecodes, deploy_debug_bytecodes);

                    if let Some(deploy_traces) = deploy_traces {
                        traces.push((TraceKind::Deployment, deploy_traces));
                    }

                    let mut tx_req = TransactionRequestFor::<FEN>::default()
                        .with_from(self.evm_opts.sender)
                        .with_input(calldata)
                        .with_nonce(sender_nonce + library_transactions.len() as u64)
                        .with_to(create2_deployer);

                    script_config.tempo.apply::<FEN::Network>(&mut tx_req, None);

                    library_transactions.push_back(BroadcastableTransaction {
                        rpc: self.evm_opts.fork_url.clone(),
                        transaction: TransactionMaybeSigned::new(tx_req),
                        contract_address: None,
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

        // HACK: if the current sender is the default script sender (which is a default value), we
        // set its nonce to a very large value before deploying the script contract. This
        // ensures that the nonce increase during this CREATE does not affect deployment
        // addresses of contracts that are deployed in the script, Otherwise, we'd have a
        // nonce mismatch during script execution and onchain simulation, potentially
        // resulting in weird errors like <https://github.com/foundry-rs/foundry/issues/8960>.
        let prev_sender_nonce = self.executor.get_nonce(self.evm_opts.sender)?;
        if self.evm_opts.sender == CALLER {
            self.executor.set_nonce(self.evm_opts.sender, u64::MAX / 2)?;
        }

        // Deploy an instance of the contract
        let DeployResult {
            address,
            raw:
                RawCallResult {
                    mut logs,
                    traces: constructor_traces,
                    debug_bytecodes: constructor_debug_bytecodes,
                    ..
                },
        } = self
            .executor
            .deploy(CALLER, code, U256::ZERO, None)
            .map_err(|err| eyre::eyre!("Failed to deploy script:\n{}", err))?;

        if self.evm_opts.sender == CALLER {
            self.executor.set_nonce(self.evm_opts.sender, prev_sender_nonce)?;
        }

        // set script address to be used by execution inspector
        if script_config.config.script_execution_protection {
            self.executor.set_script_execution(address);
        }

        traces.extend(constructor_traces.map(|traces| (TraceKind::Deployment, traces)));
        self.extend_debug_bytecodes(&mut debug_bytecodes, constructor_debug_bytecodes);

        // Optionally call the `setUp` function
        let (success, returned, exit_reason, gas_used, labeled_addresses, transactions) = if setup {
            let result = match self.executor.setup(Some(self.evm_opts.sender), address, None) {
                Ok(result) => result,
                Err(EvmError::Execution(err)) => err.raw,
                Err(err) => return Err(err.into()),
            };
            let RawCallResult {
                reverted,
                result,
                exit_reason,
                traces: setup_traces,
                labels,
                logs: setup_logs,
                gas_used,
                debug_bytecodes: setup_debug_bytecodes,
                transactions: setup_transactions,
                ..
            } = result;
            traces.extend(setup_traces.map(|traces| (TraceKind::Setup, traces)));
            logs.extend_from_slice(&setup_logs);
            self.extend_debug_bytecodes(&mut debug_bytecodes, setup_debug_bytecodes);
            if let Some(txs) = setup_transactions {
                library_transactions.extend(txs);
            }
            (!reverted, result, exit_reason, gas_used, labels, Some(library_transactions))
        } else {
            self.executor.backend_mut().set_test_contract(address);
            (true, Bytes::new(), None, 0, Default::default(), Some(library_transactions))
        };

        Ok((
            address,
            ScriptResult {
                returned,
                exit_reason,
                success,
                gas_used,
                labeled_addresses,
                debug_bytecodes: self.maybe_debug_bytecodes(debug_bytecodes),
                transactions,
                logs,
                traces,
                address: None,
                ..Default::default()
            },
        ))
    }

    /// Executes the method that will collect all broadcastable transactions.
    pub fn script(
        &mut self,
        address: Address,
        calldata: Bytes,
    ) -> Result<ScriptResult<FEN::Network>> {
        self.call(self.evm_opts.sender, address, calldata, U256::ZERO, None, false)
    }

    /// Runs a broadcastable transaction locally and persists its state.
    pub fn simulate(
        &mut self,
        transaction: &TransactionMaybeSigned<FEN::Network>,
        is_fixed_gas_limit: bool,
    ) -> Result<ScriptResult<FEN::Network>> {
        let Some(blob_hashes) = transaction.blob_versioned_hashes() else {
            return self.simulate_with_tx_env(transaction, is_fixed_gas_limit);
        };

        // Every simulated call clones the executor's tx env, so a blob transaction is run as one
        // there, making `BLOBHASH` match the published transaction.
        let original_tx_env = self.executor.tx_env().clone();
        let blob_gasprice = self.executor.evm_env().block_env.blob_gasprice().unwrap_or_default();
        let tx_env = self.executor.tx_env_mut();
        tx_env.set_blob_hashes(blob_hashes.to_vec());
        tx_env.set_max_fee_per_blob_gas(blob_gasprice);
        tx_env.set_tx_type(3);
        let result = self.simulate_with_tx_env(transaction, is_fixed_gas_limit);
        *self.executor.tx_env_mut() = original_tx_env;
        result
    }

    fn simulate_with_tx_env(
        &mut self,
        transaction: &TransactionMaybeSigned<FEN::Network>,
        is_fixed_gas_limit: bool,
    ) -> Result<ScriptResult<FEN::Network>> {
        if is_fixed_gas_limit {
            let (env, tx) = self.prepare_fixed_gas_env(transaction);
            return if tx.is_single_create() {
                let result = self.executor.deploy_with_env(env, tx, None);
                self.deployment_result(result)
            } else {
                let result = self.executor.transact_with_env(env, tx)?;
                let gas_used = result.gas_used;
                Ok(self.call_result(result, gas_used))
            };
        }

        let from = transaction.from().expect("transaction sender missing");
        if let Some(to) = transaction.to() {
            self.call(
                from,
                to,
                transaction.input().cloned().unwrap_or_default(),
                transaction.value().unwrap_or_default(),
                transaction.authorization_list(),
                true,
            )
        } else {
            let res = self.executor.deploy(
                from,
                transaction.input().cloned().expect("No data for create transaction"),
                transaction.value().unwrap_or_default(),
                None,
            );
            self.deployment_result(res)
        }
    }

    pub(crate) fn deployment_result(
        &self,
        res: Result<DeployResult<FEN>, EvmError<FEN>>,
    ) -> Result<ScriptResult<FEN::Network>> {
        let (address, RawCallResult { gas_used, logs, traces, debug_bytecodes, exit_reason, .. }) =
            match res {
                Ok(DeployResult { address, raw }) => (address, raw),
                Err(EvmError::Execution(err)) => {
                    let ExecutionErr { raw, reason } = *err;
                    sh_err!("Failed with `{reason}`:\n")?;
                    (Address::ZERO, raw)
                }
                Err(e) => {
                    eyre::bail!("Failed deploying contract: {e:?}");
                }
            };

        Ok(ScriptResult {
            returned: Bytes::new(),
            success: !address.is_zero(),
            gas_used,
            logs,
            debug_bytecodes: self.maybe_debug_bytecodes(debug_bytecodes),
            // Manually adjust gas for the trace to add back the stipend/real used gas
            traces: traces.map(|traces| vec![(TraceKind::Execution, traces)]).unwrap_or_default(),
            exit_reason,
            address: Some(address),
            ..Default::default()
        })
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
        authorization_list: Option<Vec<SignedAuthorization>>,
        commit: bool,
    ) -> Result<ScriptResult<FEN::Network>> {
        let mut res = if let Some(authorization_list) = &authorization_list {
            self.executor.call_raw_with_authorization(
                from,
                to,
                calldata.clone(),
                value,
                authorization_list.clone(),
            )?
        } else {
            self.executor.call_raw(from, to, calldata.clone(), value)?
        };
        let mut gas_used = res.gas_used;

        // We should only need to calculate realistic gas costs when preparing to broadcast
        // something. This happens during the onchain simulation stage, where we commit each
        // collected transactions.
        //
        // Otherwise don't re-execute, or some usecases might be broken: https://github.com/foundry-rs/foundry/issues/3921
        if commit {
            gas_used = self.search_optimal_gas_usage(&res, from, to, &calldata, value)?;
            res = if let Some(authorization_list) = authorization_list {
                self.executor.transact_raw_with_authorization(
                    from,
                    to,
                    calldata,
                    value,
                    authorization_list,
                )?
            } else {
                self.executor.transact_raw(from, to, calldata, value)?
            }
        }

        Ok(self.call_result(res, gas_used))
    }

    pub(crate) fn call_result(
        &self,
        res: RawCallResult<FEN>,
        gas_used: u64,
    ) -> ScriptResult<FEN::Network> {
        let RawCallResult {
            result,
            reverted,
            logs,
            traces,
            labels,
            transactions,
            debug_bytecodes,
            exit_reason,
            cheatcodes,
            ..
        } = res;
        let breakpoints = cheatcodes.map(|cheats| cheats.breakpoints).unwrap_or_default();

        ScriptResult {
            returned: result,
            success: !reverted,
            gas_used,
            logs,
            debug_bytecodes: self.maybe_debug_bytecodes(debug_bytecodes),
            traces: traces
                .map(|traces| {
                    // Manually adjust gas for the trace to add back the stipend/real used gas

                    vec![(TraceKind::Execution, traces)]
                })
                .unwrap_or_default(),
            labeled_addresses: labels,
            transactions,
            exit_reason,
            address: None,
            breakpoints,
        }
    }

    /// The executor will return the _exact_ gas value this transaction consumed, setting this value
    /// as gas limit will result in `OutOfGas` so to come up with a better estimate we search over a
    /// possible range we pick a higher gas limit 3x of a succeeded call should be safe.
    ///
    /// This might result in executing the same script multiple times. Depending on the user's goal,
    /// it might be problematic when using `ffi`.
    fn search_optimal_gas_usage(
        &mut self,
        res: &RawCallResult<FEN>,
        from: Address,
        to: Address,
        calldata: &Bytes,
        value: U256,
    ) -> Result<u64> {
        let mut gas_used = res.gas_used;
        if matches!(res.exit_reason, Some(return_ok!())) {
            // Store the current gas limit and reset it later.
            let init_gas_limit = self.executor.tx_env().gas_limit();

            let mut search = GasSearch::new(gas_used);
            while let Some(limit) = search.next_limit() {
                self.executor.tx_env_mut().set_gas_limit(limit);
                let res = self.executor.call_raw(from, to, calldata.0.clone().into(), value)?;
                search.record(limit, needs_more_gas(res.exit_reason));
            }
            gas_used = search.gas_used();
            // Reset gas limit in the executor.
            self.executor.tx_env_mut().set_gas_limit(init_gas_limit);
        }
        Ok(gas_used)
    }

    /// Prepares a fixed-limit transaction without changing the block or executor gas limit.
    pub(crate) fn prepare_fixed_gas_env(
        &self,
        transaction: &TransactionMaybeSigned<FEN::Network>,
    ) -> (EvmEnvFor<FEN>, TxEnvFor<FEN>) {
        let from = transaction.from().expect("transaction sender missing");
        let kind = transaction.to().map_or(TxKind::Create, TxKind::Call);
        let (mut env, mut tx) = self.executor.prepare_call_env(
            from,
            kind,
            transaction.input().cloned().unwrap_or_default(),
            transaction.value().unwrap_or_default(),
        );
        env.block_env.set_gas_limit(self.executor.evm_env().block_env.gas_limit());
        match transaction {
            TransactionMaybeSigned::Signed { tx: signed, .. } => {
                // Keep the complete signed payload, including access lists and network fields.
                tx = TxEnvFor::<FEN>::from_recovered_tx(signed, from);
            }
            TransactionMaybeSigned::Unsigned(request) => {
                let access_list = request.access_list().cloned().unwrap_or_default();
                if !access_list.is_empty() && tx.tx_type() == 0 {
                    tx.set_tx_type(1);
                }
                tx.set_access_list(access_list);
                if let Some(authorization_list) = transaction.authorization_list() {
                    tx.set_signed_authorization(authorization_list);
                    tx.set_tx_type(4);
                }
            }
        }
        // Retain the synthetic call's fee and chain normalization.
        tx.set_gas_price(0);
        tx.set_gas_priority_fee(None);
        tx.set_chain_id(Some(env.cfg_env.chain_id));
        tx.set_gas_limit(transaction.gas().expect("fixed gas limit missing") as u64);
        (env, tx)
    }
}

/// Gas-search arithmetic shared by ordinary and Monad simulation.
pub(crate) struct GasSearch {
    gas_used: u64,
    highest: u64,
    lowest: u64,
    last_highest: u64,
    done: bool,
}

impl GasSearch {
    pub(crate) const fn new(gas_used: u64) -> Self {
        Self {
            gas_used,
            highest: gas_used * 3,
            lowest: gas_used,
            last_highest: gas_used * 3,
            done: false,
        }
    }

    pub(crate) const fn next_limit(&self) -> Option<u64> {
        if !self.done && self.highest - self.lowest > 1 {
            Some((self.highest + self.lowest) / 2)
        } else {
            None
        }
    }

    /// Records the outcome of a probe at `limit`, where `needs_more_gas` means the limit was not
    /// enough.
    pub(crate) const fn record(&mut self, limit: u64, needs_more_gas: bool) {
        if needs_more_gas {
            self.lowest = limit;
        } else {
            self.highest = limit;
            // Stop when successive successful estimates differ by less than ten percent.
            if (self.last_highest - self.highest) * 10 / self.last_highest < 1 {
                self.gas_used = self.highest;
                self.done = true;
            } else {
                self.last_highest = self.highest;
            }
        }
    }

    pub(crate) const fn gas_used(&self) -> u64 {
        self.gas_used
    }
}

/// Returns whether the gas search treats `exit_reason` as needing more gas.
pub(crate) const fn needs_more_gas(exit_reason: Option<InstructionResult>) -> bool {
    matches!(
        exit_reason,
        Some(
            InstructionResult::Revert | InstructionResult::OutOfGas | InstructionResult::OutOfFunds
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::bytes;
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use foundry_evm::{
        backend::Backend, core::evm::TempoEvmNetwork, executors::ExecutorBuilder, revm::DatabaseRef,
    };
    use foundry_evm_networks::NetworkConfigs;
    use tempo_alloy::primitives::{TempoTransaction, TempoTxEnvelope, transaction::Call};

    #[test]
    fn successful_probes_keep_existing_ten_percent_stop() {
        let mut search = GasSearch::new(100);
        for expected in [200, 150, 125, 112, 106] {
            assert_eq!(search.next_limit(), Some(expected));
            search.record(expected, false);
        }
        assert_eq!(search.next_limit(), None);
        assert_eq!(search.gas_used(), 106);
    }

    #[test]
    fn unsuccessful_probes_keep_original_estimate() {
        let mut search = GasSearch::new(100);
        while let Some(limit) = search.next_limit() {
            search.record(limit, true);
        }
        assert_eq!(search.gas_used(), 100);
        assert_eq!(GasSearch::new(0).next_limit(), None);
    }

    #[test]
    fn only_revert_and_running_out_mean_too_little_gas() {
        for reason in
            [InstructionResult::Revert, InstructionResult::OutOfGas, InstructionResult::OutOfFunds]
        {
            assert!(needs_more_gas(Some(reason)));
        }
        for reason in [
            Some(InstructionResult::Return),
            Some(InstructionResult::Stop),
            Some(InstructionResult::InvalidFEOpcode),
            None,
        ] {
            assert!(!needs_more_gas(reason));
        }
    }

    #[test]
    fn fixed_gas_tempo_create_first_batch() {
        let signer = PrivateKeySigner::random();
        let sender = signer.address();
        let target = sender.create(0);
        // The constructor stores seven; the runtime increments that slot when called.
        let initcode = bytes!("60075f556008600e5f3960085ff35f546001015f5500");
        for batch in [false, true] {
            let mut env = EvmEnvFor::<TempoEvmNetwork>::default();
            env.cfg_env.chain_id = 31337;
            env.cfg_env.disable_nonce_check = true;
            let executor = ExecutorBuilder::<TempoEvmNetwork>::new().gas_limit(30_000_000).build(
                env,
                TxEnvFor::<TempoEvmNetwork>::default(),
                Backend::spawn(None).unwrap(),
                NetworkConfigs::with_tempo(),
            );
            let mut runner = ScriptRunner::new(executor, EvmOpts::default());
            let mut calls =
                vec![Call { to: TxKind::Create, value: U256::ZERO, input: initcode.clone() }];
            if batch {
                calls.push(Call {
                    to: TxKind::Call(target),
                    value: U256::ZERO,
                    input: Bytes::new(),
                });
            }
            let transaction = TempoTransaction {
                chain_id: 31337,
                gas_limit: 2_000_000,
                calls,
                ..Default::default()
            };
            let signature = signer.sign_hash_sync(&transaction.signature_hash()).unwrap();
            let transaction = TransactionMaybeSigned::Signed {
                tx: TempoTxEnvelope::AA(transaction.into_signed(signature.into())),
                from: sender,
            };
            let result = runner.simulate(&transaction, true).unwrap();
            assert!(result.success);
            assert_eq!(
                runner.executor.backend().storage_ref(target, U256::ZERO).unwrap(),
                U256::from(if batch { 8 } else { 7 }),
            );
            assert!(!runner.executor.is_empty_code(target).unwrap());
            assert_eq!(transaction.gas(), Some(2_000_000));
            assert_eq!(runner.executor.gas_limit(), 30_000_000);
        }
    }
}
