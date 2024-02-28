use super::{
    artifacts::ArtifactInfo,
    build::{CompiledState, LinkedBuildData, LinkedState},
    runner::{ScriptRunner, SimulationStage},
    transaction::{AdditionalContract, TransactionWithMetadata},
    ScriptArgs, ScriptConfig, ScriptResult,
};
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes, U256, U64};
use alloy_rpc_types::request::TransactionRequest;
use async_recursion::async_recursion;
use eyre::{Context, Result};
use forge::{
    backend::{Backend, DatabaseExt},
    executors::ExecutorBuilder,
    inspectors::{
        cheatcodes::{BroadcastableTransaction, BroadcastableTransactions},
        CheatsConfig,
    },
    revm::Database,
    traces::{render_trace_arena, CallTraceDecoder},
};
use foundry_cli::utils::{ensure_clean_constructor, needs_setup};
use foundry_common::{get_contract_name, provider::ethers::RpcUrl, shell, ContractsByArtifact};
use foundry_compilers::artifacts::ContractBytecodeSome;
use foundry_evm::inspectors::cheatcodes::ScriptWallets;
use futures::future::join_all;
use parking_lot::RwLock;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
};

pub struct ExecutionData {
    pub script_wallets: ScriptWallets,
    pub func: Function,
    pub calldata: Bytes,
    pub bytecode: Bytes,
    pub abi: JsonAbi,
}

pub struct PreExecutionState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
}

impl LinkedState {
    /// Given linked and compiled artifacts, prepares data we need for execution.
    pub async fn prepare_execution(self) -> Result<PreExecutionState> {
        let multi_wallet = self.args.wallets.get_multi_wallet().await?;
        let script_wallets = ScriptWallets::new(multi_wallet, self.args.evm_opts.sender);

        let ContractBytecodeSome { abi, bytecode, .. } = self.build_data.get_target_contract()?;

        let bytecode = bytecode.into_bytes().ok_or_else(|| {
            eyre::eyre!("expected fully linked bytecode, found unlinked bytecode")
        })?;

        let (func, calldata) = self.args.get_method_and_calldata(&abi)?;

        ensure_clean_constructor(&abi)?;

        Ok(PreExecutionState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            execution_data: ExecutionData { script_wallets, func, calldata, bytecode, abi },
        })
    }
}

pub struct ExecutedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_result: ScriptResult,
}

impl PreExecutionState {
    #[async_recursion]
    pub async fn execute(mut self) -> Result<ExecutedState> {
        let mut runner = self
            .prepare_runner(
                SimulationStage::Local,
                Some(self.execution_data.script_wallets.clone()),
            )
            .await?;

        self.script_config.sender_nonce = if self.script_config.evm_opts.fork_url.is_none() {
            // dapptools compatibility
            1
        } else {
            runner
                .executor
                .backend
                .basic(self.script_config.evm_opts.sender)?
                .unwrap_or_default()
                .nonce
        };

        let mut result = self.execute_with_runner(&mut runner).await?;

        // If we have a new sender from execution, we need to use it to deploy libraries and relink
        // contracts.
        if let Some(new_sender) = self.maybe_new_sender(result.transactions.as_ref())? {
            self.script_config.evm_opts.sender = new_sender;

            // Rollback to linking state to relink contracts with the new sender.
            let state = CompiledState {
                args: self.args,
                script_config: self.script_config,
                build_data: self.build_data.build_data,
            };

            return state.link()?.prepare_execution().await?.execute().await;
        }

        // Add library deployment transactions to broadcastable transactions list.
        if let Some(txs) = &mut result.transactions {
            let mut library_txs = self
                .build_data
                .predeploy_libraries
                .iter()
                .enumerate()
                .map(|(i, bytes)| BroadcastableTransaction {
                    rpc: self.script_config.evm_opts.fork_url.clone(),
                    transaction: TransactionRequest {
                        from: Some(self.script_config.evm_opts.sender),
                        input: Some(bytes.clone()).into(),
                        nonce: Some(U64::from(self.script_config.sender_nonce + i as u64)),
                        ..Default::default()
                    },
                })
                .collect::<VecDeque<_>>();

            for tx in txs.iter() {
                library_txs.push_back(BroadcastableTransaction {
                    rpc: tx.rpc.clone(),
                    transaction: tx.transaction.clone(),
                });
            }
            *txs = library_txs;
        }

        Ok(ExecutedState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            execution_data: self.execution_data,
            execution_result: result,
        })
    }

    pub async fn execute_with_runner(&self, runner: &mut ScriptRunner) -> Result<ScriptResult> {
        let (address, mut setup_result) = runner.setup(
            &self.build_data.predeploy_libraries,
            self.execution_data.bytecode.clone(),
            needs_setup(&self.execution_data.abi),
            self.script_config.sender_nonce,
            self.args.broadcast,
            self.script_config.evm_opts.fork_url.is_none(),
        )?;

        if setup_result.success {
            let script_result = runner.script(address, self.execution_data.calldata.clone())?;

            setup_result.success &= script_result.success;
            setup_result.gas_used = script_result.gas_used;
            setup_result.logs.extend(script_result.logs);
            setup_result.traces.extend(script_result.traces);
            setup_result.debug = script_result.debug;
            setup_result.labeled_addresses.extend(script_result.labeled_addresses);
            setup_result.returned = script_result.returned;
            setup_result.breakpoints = script_result.breakpoints;

            match (&mut setup_result.transactions, script_result.transactions) {
                (Some(txs), Some(new_txs)) => {
                    txs.extend(new_txs);
                }
                (None, Some(new_txs)) => {
                    setup_result.transactions = Some(new_txs);
                }
                _ => {}
            }
        }

        Ok(setup_result)
    }

    fn maybe_new_sender(
        &self,
        transactions: Option<&BroadcastableTransactions>,
    ) -> Result<Option<Address>> {
        let mut new_sender = None;

        if let Some(txs) = transactions {
            // If the user passed a `--sender` don't check anything.
            if !self.build_data.predeploy_libraries.is_empty() &&
                self.args.evm_opts.sender.is_none()
            {
                for tx in txs.iter() {
                    if tx.transaction.to.is_none() {
                        let sender = tx.transaction.from.expect("no sender");
                        if let Some(ns) = new_sender {
                            if sender != ns {
                                shell::println("You have more than one deployer who could predeploy libraries. Using `--sender` instead.")?;
                                return Ok(None);
                            }
                        } else if sender != self.script_config.evm_opts.sender {
                            new_sender = Some(sender);
                        }
                    }
                }
            }
        }
        Ok(new_sender)
    }

    /// Creates the Runner that drives script execution
    async fn prepare_runner(
        &mut self,
        stage: SimulationStage,
        script_wallets: Option<ScriptWallets>,
    ) -> Result<ScriptRunner> {
        trace!("preparing script runner");
        let env = self.script_config.evm_opts.evm_env().await?;

        let fork = if self.script_config.evm_opts.fork_url.is_some() {
            self.script_config.evm_opts.get_fork(&self.script_config.config, env.clone())
        } else {
            None
        };

        let backend = Backend::spawn(fork).await;

        // Cache forks
        if let Some(fork_url) = backend.active_fork_url() {
            self.script_config.backends.insert(fork_url.clone(), backend.clone());
        }

        // We need to enable tracing to decode contract names: local or external.
        let mut builder = ExecutorBuilder::new()
            .inspectors(|stack| stack.trace(true))
            .spec(self.script_config.config.evm_spec_id())
            .gas_limit(self.script_config.evm_opts.gas_limit());

        if let SimulationStage::Local = stage {
            builder = builder.inspectors(|stack| {
                stack.debug(self.args.debug).cheatcodes(
                    CheatsConfig::new(
                        &self.script_config.config,
                        self.script_config.evm_opts.clone(),
                        script_wallets,
                    )
                    .into(),
                )
            });
        }

        Ok(ScriptRunner::new(
            builder.build(env, backend),
            self.script_config.evm_opts.initial_balance,
            self.script_config.evm_opts.sender,
        ))
    }
}

impl ScriptArgs {
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
                .await?
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
                    return Some((*addr, info));
                }
                None
            })
            .collect();

        let mut final_txs = VecDeque::new();

        // Executes all transactions from the different forks concurrently.
        let futs = transactions
            .into_iter()
            .map(|transaction| async {
                let rpc = transaction.rpc.as_ref().expect("missing broadcastable tx rpc url");
                let mut runner = runners.get(rpc).expect("invalid rpc url").write();

                let mut tx = transaction.transaction;
                let result = runner
                    .simulate(
                        tx.from
                            .expect("transaction doesn't have a `from` address at execution time"),
                        tx.to,
                        tx.input.clone().into_input(),
                        tx.value,
                    )
                    .wrap_err("Internal EVM error during simulation")?;

                if !result.success || result.traces.is_empty() {
                    return Ok((None, result.traces));
                }

                let created_contracts = result
                    .traces
                    .iter()
                    .flat_map(|(_, traces)| {
                        traces.nodes().iter().filter_map(|node| {
                            if node.trace.kind.is_any_create() {
                                return Some(AdditionalContract {
                                    opcode: node.trace.kind,
                                    address: node.trace.address,
                                    init_code: node.trace.data.clone(),
                                });
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
                match tx.gas {
                    // If tx.gas is already set that means it was specified in script
                    Some(gas) => {
                        println!("Gas limit was set in script to {gas}");
                    }
                    // We inflate the gas used by the user specified percentage
                    None => {
                        let gas = U256::from(result.gas_used * self.gas_estimate_multiplier / 100);
                        tx.gas = Some(gas);
                    }
                }

                let tx = TransactionWithMetadata::new(
                    tx,
                    transaction.rpc,
                    &result,
                    &address_to_abi,
                    decoder,
                    created_contracts,
                    is_fixed_gas_limit,
                )?;

                eyre::Ok((Some(tx), result.traces))
            })
            .collect::<Vec<_>>();

        let mut abort = false;
        for res in join_all(futs).await {
            let (tx, traces) = res?;

            // Transaction will be `None`, if execution didn't pass.
            if tx.is_none() || script_config.evm_opts.verbosity > 3 {
                // Identify all contracts created during the call.
                if traces.is_empty() {
                    eyre::bail!(
                        "forge script requires tracing enabled to collect created contracts"
                    );
                }

                for (_, trace) in &traces {
                    println!("{}", render_trace_arena(trace, decoder).await?);
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
    async fn build_runners(
        &self,
        script_config: &ScriptConfig,
    ) -> Result<HashMap<RpcUrl, ScriptRunner>> {
        let sender = script_config.evm_opts.sender;

        if !shell::verbosity().is_silent() {
            let n = script_config.total_rpcs.len();
            let s = if n != 1 { "s" } else { "" };
            println!("\n## Setting up {n} EVM{s}.");
        }

        let futs = script_config
            .total_rpcs
            .iter()
            .map(|rpc| async {
                let mut script_config = script_config.clone();
                script_config.evm_opts.fork_url = Some(rpc.clone());
                let runner = self
                    .prepare_runner(&mut script_config, sender, SimulationStage::OnChain, None)
                    .await?;
                Ok((rpc.clone(), runner))
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
        script_wallets: Option<ScriptWallets>,
    ) -> Result<ScriptRunner> {
        trace!("preparing script runner");
        let env = script_config.evm_opts.evm_env().await?;

        // The db backend that serves all the data.
        let db = match &script_config.evm_opts.fork_url {
            Some(url) => match script_config.backends.get(url) {
                Some(db) => db.clone(),
                None => {
                    let fork = script_config.evm_opts.get_fork(&script_config.config, env.clone());
                    let backend = Backend::spawn(fork);
                    script_config.backends.insert(url.clone(), backend.clone());
                    backend
                }
            },
            None => {
                // It's only really `None`, when we don't pass any `--fork-url`. And if so, there is
                // no need to cache it, since there won't be any onchain simulation that we'd need
                // to cache the backend for.
                Backend::spawn(script_config.evm_opts.get_fork(&script_config.config, env.clone()))
            }
        };

        // We need to enable tracing to decode contract names: local or external.
        let mut builder = ExecutorBuilder::new()
            .inspectors(|stack| stack.trace(true))
            .spec(script_config.config.evm_spec_id())
            .gas_limit(script_config.evm_opts.gas_limit());

        if let SimulationStage::Local = stage {
            builder = builder.inspectors(|stack| {
                stack
                    .debug(self.debug)
                    .cheatcodes(
                        CheatsConfig::new(
                            &script_config.config,
                            script_config.evm_opts.clone(),
                            script_wallets,
                        )
                        .into(),
                    )
                    .enable_isolation(script_config.evm_opts.isolate)
            });
        }

        Ok(ScriptRunner::new(
            builder.build(env, db),
            script_config.evm_opts.initial_balance,
            sender,
        ))
    }
}
