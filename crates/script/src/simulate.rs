use super::{
    multi_sequence::MultiChainSequence,
    providers::ProvidersManager,
    runner::ScriptRunner,
    sequence::{ScriptSequence, ScriptSequenceKind},
    transaction::TransactionWithMetadata,
};
use crate::{
    broadcast::{estimate_gas, BundledState},
    build::LinkedBuildData,
    execute::{ExecutionArtifacts, ExecutionData},
    sequence::get_commit_hash,
    ScriptArgs, ScriptConfig, ScriptResult,
};
use alloy_network::TransactionBuilder;
use alloy_primitives::{utils::format_units, Address, U256};
use eyre::{Context, Result};
use foundry_cheatcodes::{BroadcastableTransactions, ScriptWallets};
use foundry_cli::utils::{has_different_gas_calc, now};
use foundry_common::{get_contract_name, provider::alloy::RpcUrl, shell, ContractData};
use foundry_evm::traces::render_trace_arena;
use futures::future::{join_all, try_join_all};
use parking_lot::RwLock;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
};

/// Same as [ExecutedState], but also contains [ExecutionArtifacts] which are obtained from
/// [ScriptResult].
///
/// Can be either converted directly to [BundledState] via [PreSimulationState::resume] or driven to
/// it through [FilledTransactionsState].
pub struct PreSimulationState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_result: ScriptResult,
    pub execution_artifacts: ExecutionArtifacts,
}

impl PreSimulationState {
    /// If simulation is enabled, simulates transactions against fork and fills gas estimation and
    /// metadata. Otherwise, metadata (e.g. additional contracts, created contract names) is
    /// left empty.
    ///
    /// Both modes will panic if any of the transactions have None for the `rpc` field.
    pub async fn fill_metadata(self) -> Result<FilledTransactionsState> {
        let transactions = if let Some(txs) = self.execution_result.transactions.as_ref() {
            if self.args.skip_simulation {
                shell::println("\nSKIPPING ON CHAIN SIMULATION.")?;
                self.no_simulation(txs.clone())?
            } else {
                self.onchain_simulation(txs.clone()).await?
            }
        } else {
            VecDeque::new()
        };

        Ok(FilledTransactionsState {
            args: self.args,
            script_config: self.script_config,
            script_wallets: self.script_wallets,
            build_data: self.build_data,
            execution_artifacts: self.execution_artifacts,
            transactions,
        })
    }

    /// Builds separate runners and environments for each RPC used in script and executes all
    /// transactions in those environments.
    ///
    /// Collects gas usage and metadata for each transaction.
    pub async fn onchain_simulation(
        &self,
        transactions: BroadcastableTransactions,
    ) -> Result<VecDeque<TransactionWithMetadata>> {
        trace!(target: "script", "executing onchain simulation");

        let runners = Arc::new(
            self.build_runners()
                .await?
                .into_iter()
                .map(|(rpc, runner)| (rpc, Arc::new(RwLock::new(runner))))
                .collect::<HashMap<_, _>>(),
        );

        let address_to_abi = self.build_address_to_abi_map();

        let mut final_txs = VecDeque::new();

        // Executes all transactions from the different forks concurrently.
        let futs = transactions
            .into_iter()
            .map(|transaction| async {
                let rpc = transaction.rpc.expect("missing broadcastable tx rpc url");
                let mut runner = runners.get(&rpc).expect("invalid rpc url").write();

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

                if !result.success {
                    return Ok((None, result.traces));
                }

                let created_contracts = result.get_created_contracts();

                // Simulate mining the transaction if the user passes `--slow`.
                if self.args.slow {
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
                        let gas = result.gas_used * self.args.gas_estimate_multiplier / 100;
                        tx.gas = Some(gas as u128);
                    }
                }
                let tx = TransactionWithMetadata::new(
                    tx,
                    rpc,
                    &result,
                    &address_to_abi,
                    &self.execution_artifacts.decoder,
                    created_contracts,
                    is_fixed_gas_limit,
                )?;

                eyre::Ok((Some(tx), result.traces))
            })
            .collect::<Vec<_>>();

        if self.script_config.evm_opts.verbosity > 3 {
            println!("==========================");
            println!("Simulated On-chain Traces:\n");
        }

        let mut abort = false;
        for res in join_all(futs).await {
            let (tx, traces) = res?;

            // Transaction will be `None`, if execution didn't pass.
            if tx.is_none() || self.script_config.evm_opts.verbosity > 3 {
                for (_, trace) in &traces {
                    println!(
                        "{}",
                        render_trace_arena(trace, &self.execution_artifacts.decoder).await?
                    );
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

    /// Build mapping from contract address to its ABI, code and contract name.
    fn build_address_to_abi_map(&self) -> BTreeMap<Address, &ContractData> {
        self.execution_artifacts
            .decoder
            .contracts
            .iter()
            .filter_map(move |(addr, contract_id)| {
                let contract_name = get_contract_name(contract_id);
                if let Ok(Some((_, data))) =
                    self.build_data.known_contracts.find_by_name_or_identifier(contract_name)
                {
                    return Some((*addr, data));
                }
                None
            })
            .collect()
    }

    /// Build [ScriptRunner] forking given RPC for each RPC used in the script.
    async fn build_runners(&self) -> Result<Vec<(RpcUrl, ScriptRunner)>> {
        let rpcs = self.execution_artifacts.rpc_data.total_rpcs.clone();
        if !shell::verbosity().is_silent() {
            let n = rpcs.len();
            let s = if n != 1 { "s" } else { "" };
            println!("\n## Setting up {n} EVM{s}.");
        }

        let futs = rpcs.into_iter().map(|rpc| async move {
            let mut script_config = self.script_config.clone();
            script_config.evm_opts.fork_url = Some(rpc.clone());
            let runner = script_config.get_runner().await?;
            Ok((rpc.clone(), runner))
        });
        try_join_all(futs).await
    }

    /// If simulation is disabled, converts transactions into [TransactionWithMetadata] type
    /// skipping metadata filling.
    fn no_simulation(
        &self,
        transactions: BroadcastableTransactions,
    ) -> Result<VecDeque<TransactionWithMetadata>> {
        Ok(transactions
            .into_iter()
            .map(|btx| {
                let mut tx = TransactionWithMetadata::from_tx_request(btx.transaction);
                tx.rpc = btx.rpc.expect("missing broadcastable tx rpc url");
                tx
            })
            .collect())
    }
}

/// At this point we have converted transactions collected during script execution to
/// [TransactionWithMetadata] objects which contain additional metadata needed for broadcasting and
/// verification.
pub struct FilledTransactionsState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub execution_artifacts: ExecutionArtifacts,
    pub transactions: VecDeque<TransactionWithMetadata>,
}

impl FilledTransactionsState {
    /// Bundles all transactions of the [`TransactionWithMetadata`] type in a list of
    /// [`ScriptSequence`]. List length will be higher than 1, if we're dealing with a multi
    /// chain deployment.
    ///
    /// Each transaction will be added with the correct transaction type and gas estimation.
    pub async fn bundle(self) -> Result<BundledState> {
        let is_multi_deployment = self.execution_artifacts.rpc_data.total_rpcs.len() > 1;

        if is_multi_deployment && !self.build_data.libraries.is_empty() {
            eyre::bail!("Multi-chain deployment is not supported with libraries.");
        }

        let mut total_gas_per_rpc: HashMap<RpcUrl, u128> = HashMap::new();

        // Batches sequence of transactions from different rpcs.
        let mut new_sequence = VecDeque::new();
        let mut manager = ProvidersManager::default();
        let mut sequences = vec![];

        // Peeking is used to check if the next rpc url is different. If so, it creates a
        // [`ScriptSequence`] from all the collected transactions up to this point.
        let mut txes_iter = self.transactions.clone().into_iter().peekable();

        while let Some(mut tx) = txes_iter.next() {
            let tx_rpc = tx.rpc.clone();
            let provider_info = manager.get_or_init_provider(&tx.rpc, self.args.legacy).await?;

            // Handles chain specific requirements.
            tx.transaction.set_chain_id(provider_info.chain);

            if !self.args.skip_simulation {
                let tx = tx.tx_mut();

                if has_different_gas_calc(provider_info.chain) {
                    trace!("estimating with different gas calculation");
                    let gas = tx.gas.expect("gas is set by simulation.");

                    // We are trying to show the user an estimation of the total gas usage.
                    //
                    // However, some transactions might depend on previous ones. For
                    // example, tx1 might deploy a contract that tx2 uses. That
                    // will result in the following `estimate_gas` call to fail,
                    // since tx1 hasn't been broadcasted yet.
                    //
                    // Not exiting here will not be a problem when actually broadcasting, because
                    // for chains where `has_different_gas_calc` returns true,
                    // we await each transaction before broadcasting the next
                    // one.
                    if let Err(err) =
                        estimate_gas(tx, &provider_info.provider, self.args.gas_estimate_multiplier)
                            .await
                    {
                        trace!("gas estimation failed: {err}");

                        // Restore gas value, since `estimate_gas` will remove it.
                        tx.set_gas_limit(gas);
                    }
                }

                let total_gas = total_gas_per_rpc.entry(tx_rpc.clone()).or_insert(0);
                *total_gas += tx.gas.expect("gas is set");
            }

            new_sequence.push_back(tx);
            // We only create a [`ScriptSequence`] object when we collect all the rpc related
            // transactions.
            if let Some(next_tx) = txes_iter.peek() {
                if next_tx.rpc == tx_rpc {
                    continue;
                }
            }

            let sequence =
                self.create_sequence(is_multi_deployment, provider_info.chain, new_sequence)?;

            sequences.push(sequence);

            new_sequence = VecDeque::new();
        }

        if !self.args.skip_simulation {
            // Present gas information on a per RPC basis.
            for (rpc, total_gas) in total_gas_per_rpc {
                let provider_info = manager.get(&rpc).expect("provider is set.");

                // We don't store it in the transactions, since we want the most updated value.
                // Right before broadcasting.
                let per_gas = if let Some(gas_price) = self.args.with_gas_price {
                    gas_price.to()
                } else {
                    provider_info.gas_price()?
                };

                shell::println("\n==========================")?;
                shell::println(format!("\nChain {}", provider_info.chain))?;

                shell::println(format!(
                    "\nEstimated gas price: {} gwei",
                    format_units(per_gas, 9)
                        .unwrap_or_else(|_| "[Could not calculate]".to_string())
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                ))?;
                shell::println(format!("\nEstimated total gas used for script: {total_gas}"))?;
                shell::println(format!(
                    "\nEstimated amount required: {} ETH",
                    format_units(total_gas.saturating_mul(per_gas), 18)
                        .unwrap_or_else(|_| "[Could not calculate]".to_string())
                        .trim_end_matches('0')
                ))?;
                shell::println("\n==========================")?;
            }
        }

        let sequence = if sequences.len() == 1 {
            ScriptSequenceKind::Single(sequences.pop().expect("empty sequences"))
        } else {
            ScriptSequenceKind::Multi(MultiChainSequence::new(
                sequences,
                &self.args.sig,
                &self.build_data.build_data.target,
                &self.script_config.config,
                !self.args.broadcast,
            )?)
        };

        Ok(BundledState {
            args: self.args,
            script_config: self.script_config,
            script_wallets: self.script_wallets,
            build_data: self.build_data,
            sequence,
        })
    }

    /// Creates a [ScriptSequence] object from the given transactions.
    fn create_sequence(
        &self,
        multi: bool,
        chain: u64,
        transactions: VecDeque<TransactionWithMetadata>,
    ) -> Result<ScriptSequence> {
        // Paths are set to None for multi-chain sequences parts, because they don't need to be
        // saved to a separate file.
        let paths = if multi {
            None
        } else {
            Some(ScriptSequence::get_paths(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.build_data.target,
                chain,
                !self.args.broadcast,
            )?)
        };

        let commit = get_commit_hash(&self.script_config.config.__root.0);

        let libraries = self
            .build_data
            .libraries
            .libs
            .iter()
            .flat_map(|(file, libs)| {
                libs.iter()
                    .map(|(name, address)| format!("{}:{name}:{address}", file.to_string_lossy()))
            })
            .collect();

        Ok(ScriptSequence {
            transactions,
            returns: self.execution_artifacts.returns.clone(),
            receipts: vec![],
            pending: vec![],
            paths,
            timestamp: now().as_secs(),
            libraries,
            chain,
            commit,
        })
    }
}
