use super::{
    artifacts::ArtifactInfo,
    runner::SimulationStage,
    transaction::{AdditionalContract, TransactionWithMetadata},
    *,
};
use alloy_primitives::{Address, Bytes, U256};
use ethers::types::transaction::eip2718::TypedTransaction;
use eyre::Result;
use forge::{
    backend::Backend,
    executors::ExecutorBuilder,
    inspectors::{cheatcodes::util::BroadcastableTransactions, CheatsConfig},
    traces::{CallTraceDecoder, Traces},
    utils::CallKind,
};
use foundry_cli::utils::{ensure_clean_constructor, needs_setup};
use foundry_common::{shell, RpcUrl};
use foundry_compilers::artifacts::CompactContractBytecode;
use foundry_utils::types::ToEthers;
use futures::future::join_all;
use parking_lot::RwLock;
use std::{collections::VecDeque, sync::Arc};
use tracing::trace;

/// Helper alias type for the processed result of a runner onchain simulation.
type RunnerResult = (Option<TransactionWithMetadata>, Traces);

impl ScriptArgs {
    /// Locally deploys and executes the contract method that will collect all broadcastable
    /// transactions.
    pub async fn execute(
        &self,
        script_config: &mut ScriptConfig,
        contract: CompactContractBytecode,
        sender: Address,
        predeploy_libraries: &[Bytes],
    ) -> Result<ScriptResult> {
        trace!(target: "script", "start executing script");

        let CompactContractBytecode { abi, bytecode, .. } = contract;

        let abi = abi.ok_or_else(|| eyre::eyre!("no ABI found for contract"))?;
        let bytecode = bytecode
            .ok_or_else(|| eyre::eyre!("no bytecode found for contract"))?
            .object
            .into_bytes()
            .ok_or_else(|| {
                eyre::eyre!("expected fully linked bytecode, found unlinked bytecode")
            })?;

        ensure_clean_constructor(&abi)?;

        let mut runner = self.prepare_runner(script_config, sender, SimulationStage::Local).await;
        let (address, mut result) = runner.setup(
            predeploy_libraries,
            bytecode.0.into(),
            needs_setup(&abi),
            script_config.sender_nonce,
            self.broadcast,
            script_config.evm_opts.fork_url.is_none(),
        )?;

        let (func, calldata) = self.get_method_and_calldata(&abi)?;
        script_config.called_function = Some(func);

        // Only call the method if `setUp()` succeeded.
        if result.success {
            let script_result = runner.script(address, calldata.0.into())?;

            result.success &= script_result.success;
            result.gas_used = script_result.gas_used;
            result.logs.extend(script_result.logs);
            result.traces.extend(script_result.traces);
            result.debug = script_result.debug;
            result.labeled_addresses.extend(script_result.labeled_addresses);
            result.returned = script_result.returned;
            result.script_wallets.extend(script_result.script_wallets);
            result.breakpoints = script_result.breakpoints;

            match (&mut result.transactions, script_result.transactions) {
                (Some(txs), Some(new_txs)) => {
                    txs.extend(new_txs);
                }
                (None, Some(new_txs)) => {
                    result.transactions = Some(new_txs);
                }
                _ => {}
            }
        }

        Ok(result)
    }

    /// Simulates onchain state by executing a list of transactions locally and persisting their
    /// state. Returns the transactions and any CREATE2 contract address created.
    pub async fn onchain_simulation(
        &self,
        transactions: BroadcastableTransactions,
        script_config: &ScriptConfig,
        decoder: &CallTraceDecoder,
        contracts: &ContractsByArtifact,
    ) -> Result<VecDeque<TransactionWithMetadata>> {
        trace!(target: "script", "executing onchain simulation");

        let runners = Arc::new(
            self.build_runners(script_config)
                .await
                .into_iter()
                .map(|(rpc, runner)| (rpc, Arc::new(RwLock::new(runner))))
                .collect::<HashMap<_, _>>(),
        );

        if script_config.evm_opts.verbosity > 3 {
            println!("==========================");
            println!("Simulated On-chain Traces:\n");
        }

        let address_to_abi: BTreeMap<Address, ArtifactInfo> = decoder
            .contracts
            .iter()
            .filter_map(|(addr, contract_id)| {
                let contract_name = get_contract_name(contract_id);
                if let Ok(Some((_, (abi, code)))) =
                    contracts.find_by_name_or_identifier(contract_name)
                {
                    let info = ArtifactInfo {
                        contract_name: contract_name.to_string(),
                        contract_id: contract_id.to_string(),
                        abi,
                        code,
                    };
                    return Some((*addr, info))
                }
                None
            })
            .collect();

        let mut final_txs = VecDeque::new();

        // Executes all transactions from the different forks concurrently.
        let futs =
            transactions
                .into_iter()
                .map(|transaction| async {
                    let mut runner = runners
                        .get(transaction.rpc.as_ref().expect("to have been filled already."))
                        .expect("to have been built.")
                        .write();

                    if let TypedTransaction::Legacy(mut tx) = transaction.transaction {
                        let result = runner
                        .simulate(
                            tx.from.expect(
                                "Transaction doesn't have a `from` address at execution time",
                            ).to_alloy(),
                            tx.to.clone(),
                            tx.data.clone().map(|b| b.0.into()),
                            tx.value.map(|v| v.to_alloy()),
                        )
                        .expect("Internal EVM error");

                        if !result.success || result.traces.is_empty() {
                            return Ok((None, result.traces))
                        }

                        let created_contracts = result
                            .traces
                            .iter()
                            .flat_map(|(_, traces)| {
                                traces.arena.iter().filter_map(|node| {
                                    if matches!(node.kind(), CallKind::Create | CallKind::Create2) {
                                        return Some(AdditionalContract {
                                            opcode: node.kind(),
                                            address: node.trace.address,
                                            init_code: node.trace.data.to_raw(),
                                        })
                                    }
                                    None
                                })
                            })
                            .collect();

                        // Simulate mining the transaction if the user passes `--slow`.
                        if self.slow {
                            runner.executor.env.block.number += U256::from(1);
                        }

                        let is_fixed_gas_limit = tx.gas.is_some();
                        // If tx.gas is already set that means it was specified in script
                        if !is_fixed_gas_limit {
                            // We inflate the gas used by the user specified percentage
                            tx.gas = Some(
                                U256::from(result.gas_used * self.gas_estimate_multiplier / 100)
                                    .to_ethers(),
                            );
                        } else {
                            println!("Gas limit was set in script to {:}", tx.gas.unwrap());
                        }

                        let tx = TransactionWithMetadata::new(
                            tx.into(),
                            transaction.rpc,
                            &result,
                            &address_to_abi,
                            decoder,
                            created_contracts,
                            is_fixed_gas_limit,
                        )?;

                        Ok((Some(tx), result.traces))
                    } else {
                        unreachable!()
                    }
                })
                .collect::<Vec<_>>();

        let mut abort = false;
        for res in join_all(futs).await {
            // type hint
            let res: Result<RunnerResult> = res;

            let (tx, mut traces) = res?;

            // Transaction will be `None`, if execution didn't pass.
            if tx.is_none() || script_config.evm_opts.verbosity > 3 {
                // Identify all contracts created during the call.
                if traces.is_empty() {
                    eyre::bail!(
                        "Forge script requires tracing enabled to collect created contracts."
                    )
                }

                for (_kind, trace) in &mut traces {
                    decoder.decode(trace).await;
                    println!("{trace}");
                }
            }

            if let Some(tx) = tx {
                final_txs.push_back(tx);
            } else {
                abort = true;
            }
        }

        if abort {
            eyre::bail!("Simulated execution failed.")
        }

        Ok(final_txs)
    }

    /// Build the multiple runners from different forks.
    async fn build_runners(&self, script_config: &ScriptConfig) -> HashMap<RpcUrl, ScriptRunner> {
        let sender = script_config.evm_opts.sender;

        if !shell::verbosity().is_silent() {
            eprintln!("\n## Setting up ({}) EVMs.", script_config.total_rpcs.len());
        }

        let futs = script_config
            .total_rpcs
            .iter()
            .map(|rpc| async {
                let mut script_config = script_config.clone();
                script_config.evm_opts.fork_url = Some(rpc.clone());

                (
                    rpc.clone(),
                    self.prepare_runner(&mut script_config, sender, SimulationStage::OnChain).await,
                )
            })
            .collect::<Vec<_>>();

        join_all(futs).await.into_iter().collect()
    }

    /// Creates the Runner that drives script execution
    async fn prepare_runner(
        &self,
        script_config: &mut ScriptConfig,
        sender: Address,
        stage: SimulationStage,
    ) -> ScriptRunner {
        trace!("preparing script runner");
        let env =
            script_config.evm_opts.evm_env().await.expect("Could not instantiate fork environment");

        // The db backend that serves all the data.
        let db = match &script_config.evm_opts.fork_url {
            Some(url) => match script_config.backends.get(url) {
                Some(db) => db.clone(),
                None => {
                    let backend = Backend::spawn(
                        script_config.evm_opts.get_fork(&script_config.config, env.clone()),
                    )
                    .await;
                    script_config.backends.insert(url.clone(), backend);
                    script_config.backends.get(url).unwrap().clone()
                }
            },
            None => {
                // It's only really `None`, when we don't pass any `--fork-url`. And if so, there is
                // no need to cache it, since there won't be any onchain simulation that we'd need
                // to cache the backend for.
                Backend::spawn(script_config.evm_opts.get_fork(&script_config.config, env.clone()))
                    .await
            }
        };

        // We need to enable tracing to decode contract names: local or external.
        let mut builder = ExecutorBuilder::new()
            .inspectors(|stack| stack.trace(true))
            .spec(script_config.config.evm_spec_id())
            .gas_limit(script_config.evm_opts.gas_limit());

        if let SimulationStage::Local = stage {
            builder = builder.inspectors(|stack| {
                stack.debug(self.debug).cheatcodes(
                    CheatsConfig::new(&script_config.config, &script_config.evm_opts).into(),
                )
            });
        }

        ScriptRunner::new(builder.build(env, db), script_config.evm_opts.initial_balance, sender)
    }
}
