use super::{
    multi_sequence::MultiChainSequence, providers::ProvidersManager, runner::ScriptRunner,
    sequence::ScriptSequenceKind, transaction::ScriptTransactionBuilder,
};
use crate::{
    ScriptArgs, ScriptConfig, ScriptResult,
    broadcast::{BundledState, estimate_gas},
    build::LinkedBuildData,
    execute::{ExecutionArtifacts, ExecutionData, build_trace_decoder_for_context},
    sequence::get_commit_hash,
};
use alloy_chains::{Chain, NamedChain};
use alloy_evm::revm::context::Block;
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, U256, map::HashMap, utils::format_units};
use alloy_provider::Provider;
use dialoguer::Confirm;
use eyre::{Context, Result};
use forge_script_sequence::{ScriptSequence, TransactionWithMetadata};
use foundry_cheatcodes::Wallets;
use foundry_cli::utils::{has_different_gas_calc, now};
use foundry_common::{
    ContractData, ContractsByArtifact, provider::fee::resolve_broadcast_eip1559_fees, shell,
    tempo::known_fee_token_symbol,
};
use foundry_evm::{
    core::{FoundryBlock, evm::FoundryEvmNetwork},
    traces::{
        CallTraceDecoder, debug::ContractSources, decode_trace_arena, prune_trace_depth,
        render_trace_arena_inner,
    },
};
use foundry_wallets::wallet_browser::signer::BrowserSigner;
use futures::future::join_all;
use parking_lot::RwLock;
use std::{
    collections::{BTreeMap, VecDeque},
    mem,
    sync::Arc,
};

/// Same as [ExecutedState](crate::execute::ExecutedState), but also contains [ExecutionArtifacts]
/// which are obtained from [ScriptResult].
///
/// Can be either converted directly to [BundledState] or driven to it through
/// [FilledTransactionsState].
pub struct PreSimulationState<FEN: FoundryEvmNetwork> {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig<FEN>,
    pub script_wallets: Wallets,
    pub browser_wallet: Option<BrowserSigner<FEN::Network>>,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_result: ScriptResult<FEN::Network>,
    pub execution_artifacts: ExecutionArtifacts,
}

struct RpcSimulationContext<R> {
    runner: RwLock<R>,
    decoder: CallTraceDecoder,
}

enum RpcContexts<R> {
    Simulation(Arc<HashMap<String, RpcSimulationContext<R>>>),
    Decoding(HashMap<String, CallTraceDecoder>),
}

impl<R> RpcContexts<R> {
    fn decoder(&self, rpc: &str) -> &CallTraceDecoder {
        match self {
            Self::Simulation(contexts) => &context_for_rpc(contexts, rpc).decoder,
            Self::Decoding(decoders) => decoders.get(rpc).expect("invalid rpc url"),
        }
    }
}

fn context_for_rpc<'a, R>(
    contexts: &'a HashMap<String, RpcSimulationContext<R>>,
    rpc: &str,
) -> &'a RpcSimulationContext<R> {
    contexts.get(rpc).expect("invalid rpc url")
}

async fn build_rpc_simulation_context<FEN: FoundryEvmNetwork>(
    rpc: String,
    args: &ScriptArgs,
    script_config: &ScriptConfig<FEN>,
    known_contracts: &ContractsByArtifact,
    sources: &ContractSources,
    execution_result: &ScriptResult<FEN::Network>,
) -> Result<(String, RpcSimulationContext<ScriptRunner<FEN>>)> {
    let mut script_config = script_config.clone();
    script_config.set_fork_url(rpc.clone());
    let mut runner = script_config._get_runner(None, false, false).await?;
    runner.executor.enable_block_context_progression()?;
    let decoder = build_trace_decoder_for_context(
        args,
        &script_config,
        known_contracts,
        sources,
        execution_result,
        script_config.source_chain_id.map(Chain::from),
    )?;
    Ok((rpc, RpcSimulationContext { runner: RwLock::new(runner), decoder }))
}

async fn build_rpc_decoder<FEN: FoundryEvmNetwork>(
    rpc: String,
    args: &ScriptArgs,
    script_config: &ScriptConfig<FEN>,
    known_contracts: &ContractsByArtifact,
    sources: &ContractSources,
    execution_result: &ScriptResult<FEN::Network>,
) -> Result<(String, CallTraceDecoder)> {
    let mut script_config = script_config.clone();
    script_config.set_fork_url(rpc.clone());
    let _ = script_config.resolve_execution_env().await?;
    let decoder = build_trace_decoder_for_context(
        args,
        &script_config,
        known_contracts,
        sources,
        execution_result,
        script_config.source_chain_id.map(Chain::from),
    )?;
    Ok((rpc, decoder))
}

impl<FEN: FoundryEvmNetwork> PreSimulationState<FEN> {
    /// If simulation is enabled, simulates transactions against the fork and fills gas estimation
    /// and execution metadata. Otherwise, fills metadata that can be derived without transaction
    /// simulation using each RPC's resolved execution context.
    ///
    /// Both modes will panic if any of the transactions have None for the `rpc` field.
    pub async fn fill_metadata(self) -> Result<FilledTransactionsState<FEN>> {
        let address_to_abi = self.build_address_to_abi_map();
        let contexts = if self.args.skip_simulation {
            RpcContexts::Decoding(self.build_rpc_decoders().await?)
        } else {
            RpcContexts::Simulation(Arc::new(
                self.build_runners().await?.into_iter().collect::<HashMap<_, _>>(),
            ))
        };

        let mut transactions = self
            .execution_result
            .transactions
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|tx| {
                let rpc = tx.rpc.expect("missing broadcastable tx rpc url");
                let sender = tx.transaction.from().expect("all transactions should have a sender");
                let nonce = tx.transaction.nonce().expect("all transactions should have a nonce");
                let to = tx.transaction.to();
                let decoder = contexts.decoder(&rpc);

                let mut builder = ScriptTransactionBuilder::new(tx.transaction, rpc);

                if to.is_some() {
                    builder.set_call(
                        &address_to_abi,
                        decoder,
                        self.script_config.evm_opts.create2_deployer,
                    )?;
                } else {
                    builder.set_create(false, sender.create(nonce), &address_to_abi)?;
                }

                Ok(builder.build())
            })
            .collect::<Result<VecDeque<_>>>()?;

        match contexts {
            RpcContexts::Simulation(contexts) => {
                transactions = self.simulate_and_fill_with_contexts(transactions, contexts).await?;
            }
            RpcContexts::Decoding(_) => sh_println!("\nSKIPPING ON CHAIN SIMULATION.")?,
        }

        Ok(FilledTransactionsState {
            args: self.args,
            script_config: self.script_config,
            script_wallets: self.script_wallets,
            browser_wallet: self.browser_wallet,
            build_data: self.build_data,
            execution_artifacts: self.execution_artifacts,
            transactions,
        })
    }

    /// Executes every transaction in its RPC-specific simulation context and collects gas usage
    /// and metadata.
    async fn simulate_and_fill_with_contexts(
        &self,
        transactions: VecDeque<TransactionWithMetadata<FEN::Network>>,
        contexts: Arc<HashMap<String, RpcSimulationContext<ScriptRunner<FEN>>>>,
    ) -> Result<VecDeque<TransactionWithMetadata<FEN::Network>>> {
        trace!(target: "script", "executing onchain simulation");

        let mut final_txs = VecDeque::new();

        // Executes all transactions from the different forks concurrently.
        let futs = transactions
            .into_iter()
            .map(|mut transaction| async {
                let rpc = transaction.rpc.clone();
                let context = context_for_rpc(&contexts, &rpc);
                let mut runner = context.runner.write();
                let tx = transaction.tx_mut();

                let to = tx.to();
                let result = runner
                    .simulate(
                        tx.from()
                            .expect("transaction doesn't have a `from` address at execution time"),
                        to,
                        tx.input().cloned(),
                        tx.value(),
                        tx.authorization_list(),
                    )
                    .wrap_err("Internal EVM error during simulation")?;

                if !result.success {
                    return Ok((rpc, None, false, result.traces));
                }

                // Simulate mining the transaction if the user passes `--slow`.
                if self.args.slow {
                    runner.executor.advance_block_context();
                    let block_number = runner.executor.evm_env().block_env.number() + U256::from(1);
                    runner.executor.evm_env_mut().block_env.set_number(block_number);
                }

                let is_noop_tx = if let Some(to) = to {
                    runner.executor.is_empty_code(to)? && tx.value().unwrap_or_default().is_zero()
                } else {
                    false
                };

                let transaction = ScriptTransactionBuilder::from(transaction)
                    .with_execution_result(
                        &result,
                        self.args.gas_estimate_multiplier,
                        &self.build_data,
                    )
                    .build();

                eyre::Ok((rpc, Some(transaction), is_noop_tx, result.traces))
            })
            .collect::<Vec<_>>();

        let tracing = &self.script_config.config.tracing;
        if !shell::is_json() && tracing.verbosity > 3 {
            sh_println!("==========================")?;
            sh_println!("Simulated On-chain Traces:\n")?;
        }

        let mut abort = false;
        for res in join_all(futs).await {
            let (rpc, tx, is_noop_tx, mut traces) = res?;

            // Transaction will be `None`, if execution didn't pass.
            if !shell::is_json() && (tx.is_none() || tracing.verbosity > 3) {
                let decoder = &context_for_rpc(&contexts, &rpc).decoder;
                for (_, trace) in &mut traces {
                    decode_trace_arena(trace, decoder).await;
                    if let Some(trace_depth) = tracing.trace_depth {
                        prune_trace_depth(trace, trace_depth);
                    }
                    sh_println!(
                        "{}",
                        render_trace_arena_inner(trace, false, tracing.verbosity > 4)
                    )?;
                }
            }

            if let Some(tx) = tx {
                if is_noop_tx {
                    let to = tx.contract_address.unwrap();
                    sh_warn!(
                        "Script contains a transaction to {to} which does not contain any code."
                    )?;

                    // Only prompt if we're broadcasting and we've not disabled interactivity.
                    if self.args.should_broadcast()
                        && !self.args.non_interactive
                        && !Confirm::new()
                            .with_prompt("Do you wish to continue?".to_string())
                            .interact()?
                    {
                        eyre::bail!("User canceled the script.");
                    }
                }

                final_txs.push_back(tx);
            } else {
                abort = true;
            }
        }

        if abort {
            eyre::bail!("Simulated execution failed.");
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
                if let Ok(Some((_, data))) =
                    self.build_data.known_contracts.find_by_name_or_identifier(contract_id)
                {
                    return Some((*addr, data));
                }
                None
            })
            .collect()
    }

    /// Build [ScriptRunner] forking given RPC for each RPC used in the script.
    async fn build_runners(
        &self,
    ) -> Result<Vec<(String, RpcSimulationContext<ScriptRunner<FEN>>)>> {
        let rpcs = &self.execution_artifacts.rpc_data.total_rpcs;

        if !shell::is_json() {
            let n = rpcs.len();
            let s = if n == 1 { "" } else { "s" };
            sh_println!("\n## Setting up {n} EVM{s}.")?;
        }

        // Context construction performs several identity and block probes per endpoint. Resolve
        // endpoints serially so setup does not create an unbounded cross-endpoint request burst.
        let mut contexts = Vec::with_capacity(rpcs.len());
        for rpc in rpcs.iter().cloned() {
            contexts.push(
                build_rpc_simulation_context(
                    rpc,
                    &self.args,
                    &self.script_config,
                    &self.build_data.known_contracts,
                    &self.build_data.sources,
                    &self.execution_result,
                )
                .await?,
            );
        }
        Ok(contexts)
    }

    /// Builds one trace decoder for every RPC without constructing simulation runners.
    async fn build_rpc_decoders(&self) -> Result<HashMap<String, CallTraceDecoder>> {
        let rpcs = &self.execution_artifacts.rpc_data.total_rpcs;
        // Decoder construction resolves the same endpoint context as a simulation runner.
        let mut decoders = HashMap::default();
        for rpc in rpcs.iter().cloned() {
            let (rpc, decoder) = build_rpc_decoder(
                rpc,
                &self.args,
                &self.script_config,
                &self.build_data.known_contracts,
                &self.build_data.sources,
                &self.execution_result,
            )
            .await?;
            decoders.insert(rpc, decoder);
        }
        Ok(decoders)
    }
}

#[cfg(all(test, feature = "monad"))]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use anvil::{NodeConfig, spawn};
    use foundry_cli::opts::TempoOpts;
    use foundry_config::Config;
    use foundry_evm::{
        core::{evm::MonadEvmNetwork, opts::EvmOpts},
        executors::ExecutorBuilder,
        hardforks::MonadHardfork,
    };
    use foundry_evm_networks::NetworkConfigs;

    const RESERVE_BALANCE_ADDRESS: Address = address!("0000000000000000000000000000000000001001");

    #[tokio::test(flavor = "multi_thread")]
    async fn multi_rpc_fork_selects_trace_decoder_for_source_hardfork() {
        let (monad_eight_api, monad_eight) = spawn(
            NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_hardfork(Some(MonadHardfork::MonadEight.into())),
        )
        .await;
        let (monad_nine_api, monad_nine) = spawn(
            NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_hardfork(Some(MonadHardfork::MonadNine.into())),
        )
        .await;
        monad_eight_api.mine_one().await.unwrap();
        monad_nine_api.mine_one().await.unwrap();
        let monad_eight_rpc = monad_eight.http_endpoint();
        let monad_nine_rpc = monad_nine.http_endpoint();

        let mut evm_opts = EvmOpts {
            fork_url: Some(monad_eight_rpc.clone()),
            fork_block_number: Some(0),
            networks: NetworkConfigs::with_monad(),
            ..Default::default()
        };
        evm_opts.env.chain_id = Some(42);
        let script_config = ScriptConfig::<MonadEvmNetwork>::new(
            Config::default(),
            evm_opts,
            ExecutorBuilder::<MonadEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            Some(0),
        )
        .await
        .unwrap();
        let args = ScriptArgs::default();
        let known_contracts = ContractsByArtifact::default();
        let sources = ContractSources::default();
        let execution_result = ScriptResult::default();
        let contexts = [
            build_rpc_simulation_context(
                monad_eight_rpc.clone(),
                &args,
                &script_config,
                &known_contracts,
                &sources,
                &execution_result,
            )
            .await
            .unwrap(),
            build_rpc_simulation_context(
                monad_nine_rpc.clone(),
                &args,
                &script_config,
                &known_contracts,
                &sources,
                &execution_result,
            )
            .await
            .unwrap(),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

        let monad_eight = context_for_rpc(&contexts, &monad_eight_rpc);
        let monad_eight_runner = monad_eight.runner.read();
        assert_eq!(monad_eight_runner.executor.evm_env().cfg_env.chain_id, 42);
        assert_eq!(monad_eight_runner.executor.evm_env().block_env.number(), U256::ZERO);
        assert_eq!(monad_eight_runner.evm_opts.fork_block_number, Some(0));
        assert!(!monad_eight_runner.evm_opts.fork_block_number_is_inferred);
        assert_eq!(monad_eight_runner.evm_opts.networks, NetworkConfigs::with_monad());
        assert!(!monad_eight_runner.evm_opts.fork_network_is_inferred);
        assert_eq!(monad_eight.decoder.chain_id, Some(NamedChain::Monad as u64));
        assert_eq!(monad_eight.decoder.hardfork(), Some(MonadHardfork::MonadEight.into()));
        assert!(!monad_eight.decoder.precompile_labels().contains_key(&RESERVE_BALANCE_ADDRESS));

        let monad_nine = context_for_rpc(&contexts, &monad_nine_rpc);
        let monad_nine_runner = monad_nine.runner.read();
        assert_eq!(monad_nine_runner.executor.evm_env().cfg_env.chain_id, 42);
        assert_eq!(monad_nine_runner.executor.evm_env().block_env.number(), U256::ZERO);
        assert_eq!(monad_nine_runner.evm_opts.fork_block_number, Some(0));
        assert!(!monad_nine_runner.evm_opts.fork_block_number_is_inferred);
        assert_eq!(monad_nine_runner.evm_opts.networks, NetworkConfigs::with_monad());
        assert!(!monad_nine_runner.evm_opts.fork_network_is_inferred);
        assert_eq!(monad_nine.decoder.chain_id, Some(NamedChain::Monad as u64));
        assert_eq!(monad_nine.decoder.hardfork(), Some(MonadHardfork::MonadNine.into()));
        assert_eq!(
            monad_nine.decoder.precompile_labels().get(&RESERVE_BALANCE_ADDRESS),
            Some(&"ReserveBalance".to_string())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn skip_simulation_fork_builds_per_rpc_trace_decoders() {
        let (monad_eight_api, monad_eight) = spawn(
            NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_hardfork(Some(MonadHardfork::MonadEight.into())),
        )
        .await;
        let (monad_nine_api, monad_nine) = spawn(
            NodeConfig::test_monad()
                .with_chain_id(Some(NamedChain::Monad as u64))
                .with_hardfork(Some(MonadHardfork::MonadNine.into())),
        )
        .await;
        monad_eight_api.mine_one().await.unwrap();
        monad_nine_api.mine_one().await.unwrap();
        let monad_eight_rpc = monad_eight.http_endpoint();
        let monad_nine_rpc = monad_nine.http_endpoint();

        let script_config = ScriptConfig::<MonadEvmNetwork>::new(
            Config::default(),
            EvmOpts {
                fork_url: Some(monad_eight_rpc.clone()),
                fork_block_number: Some(0),
                networks: NetworkConfigs::with_monad(),
                ..Default::default()
            },
            ExecutorBuilder::<MonadEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            Some(0),
        )
        .await
        .unwrap();
        let args = ScriptArgs { skip_simulation: true, ..Default::default() };
        let known_contracts = ContractsByArtifact::default();
        let sources = ContractSources::default();
        let execution_result = ScriptResult::default();
        let decoders = [
            build_rpc_decoder(
                monad_eight_rpc.clone(),
                &args,
                &script_config,
                &known_contracts,
                &sources,
                &execution_result,
            )
            .await
            .unwrap(),
            build_rpc_decoder(
                monad_nine_rpc.clone(),
                &args,
                &script_config,
                &known_contracts,
                &sources,
                &execution_result,
            )
            .await
            .unwrap(),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();

        let monad_eight = decoders.get(&monad_eight_rpc).unwrap();
        assert_eq!(monad_eight.hardfork(), Some(MonadHardfork::MonadEight.into()));
        assert!(!monad_eight.precompile_labels().contains_key(&RESERVE_BALANCE_ADDRESS));

        let monad_nine = decoders.get(&monad_nine_rpc).unwrap();
        assert_eq!(monad_nine.hardfork(), Some(MonadHardfork::MonadNine.into()));
        assert_eq!(
            monad_nine.precompile_labels().get(&RESERVE_BALANCE_ADDRESS),
            Some(&"ReserveBalance".to_string())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multi_rpc_fork_rejects_inferred_network_change() {
        let (_monad_api, monad) = spawn(NodeConfig::test_monad()).await;
        let (_ethereum_api, ethereum) = spawn(NodeConfig::test()).await;
        let ethereum_rpc = ethereum.http_endpoint();
        let script_config = ScriptConfig::<MonadEvmNetwork>::new(
            Config::default(),
            EvmOpts { fork_url: Some(monad.http_endpoint()), ..Default::default() },
            ExecutorBuilder::<MonadEvmNetwork>::new(),
            false,
            TempoOpts::default(),
            Some(0),
        )
        .await
        .unwrap();
        assert!(script_config.evm_opts.fork_network_is_inferred);

        let result = build_rpc_simulation_context(
            ethereum_rpc.clone(),
            &ScriptArgs::default(),
            &script_config,
            &ContractsByArtifact::default(),
            &ContractSources::default(),
            &ScriptResult::default(),
        )
        .await;
        let Err(error) = result else { panic!("inferred cross-network fork should be rejected") };
        assert!(
            error
                .to_string()
                .contains("fork network `ethereum` is incompatible with the active EVM"),
            "{error}"
        );

        let result = build_rpc_decoder(
            ethereum_rpc,
            &ScriptArgs { skip_simulation: true, ..Default::default() },
            &script_config,
            &ContractsByArtifact::default(),
            &ContractSources::default(),
            &ScriptResult::default(),
        )
        .await;
        let Err(error) = result else {
            panic!("skip-simulation decoder should reject an inferred cross-network fork")
        };
        assert!(
            error
                .to_string()
                .contains("fork network `ethereum` is incompatible with the active EVM"),
            "{error}"
        );
    }
}

/// At this point we have converted transactions collected during script execution to
/// [TransactionWithMetadata] objects which contain additional metadata needed for broadcasting and
/// verification.
pub struct FilledTransactionsState<FEN: FoundryEvmNetwork> {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig<FEN>,
    pub script_wallets: Wallets,
    pub browser_wallet: Option<BrowserSigner<FEN::Network>>,
    pub build_data: LinkedBuildData,
    pub execution_artifacts: ExecutionArtifacts,
    pub transactions: VecDeque<TransactionWithMetadata<FEN::Network>>,
}

impl<FEN: FoundryEvmNetwork> FilledTransactionsState<FEN> {
    /// Bundles all transactions of the [`TransactionWithMetadata`] type in a list of
    /// [`ScriptSequence`]. List length will be higher than 1, if we're dealing with a multi
    /// chain deployment.
    ///
    /// Each transaction will be added with the correct transaction type and gas estimation.
    pub async fn bundle(mut self) -> Result<BundledState<FEN>> {
        let is_multi_deployment = self.execution_artifacts.rpc_data.total_rpcs.len() > 1;

        if is_multi_deployment && !self.build_data.libraries.is_empty() {
            eyre::bail!("Multi-chain deployment is not supported with libraries.");
        }

        let mut total_gas_per_rpc: HashMap<String, u128> = HashMap::default();

        // Batches sequence of transactions from different rpcs.
        let mut new_sequence = VecDeque::new();
        let mut manager = ProvidersManager::<FEN::Network>::default();
        let mut sequences = vec![];

        // Peeking is used to check if the next rpc url is different. If so, it creates a
        // [`ScriptSequence`] from all the collected transactions up to this point.
        let mut txes_iter = mem::take(&mut self.transactions).into_iter().peekable();

        while let Some(mut tx) = txes_iter.next() {
            let tx_rpc = tx.rpc.clone();
            let provider_info = manager
                .get_or_init_provider(
                    &tx.rpc,
                    self.execution_artifacts.rpc_data.chain_ids.get(&tx.rpc).copied(),
                    self.args.legacy,
                    self.script_config.config.eip1559_fee_estimate,
                    &self.script_config.config,
                )
                .await?;

            if let Some(tx) = tx.tx_mut().as_unsigned_mut() {
                // Handles chain specific requirements for unsigned transactions.
                tx.set_chain_id(provider_info.chain);
            }

            if !self.args.skip_simulation {
                let tx = tx.tx_mut();

                if has_different_gas_calc(provider_info.chain) {
                    // only estimate gas for unsigned transactions
                    if let Some(tx) = tx.as_unsigned_mut() {
                        trace!("estimating with different gas calculation");
                        let gas = tx.gas_limit().expect("gas is set by simulation.");

                        // We are trying to show the user an estimation of the total gas usage.
                        //
                        // However, some transactions might depend on previous ones. For
                        // example, tx1 might deploy a contract that tx2 uses. That
                        // will result in the following `estimate_gas` call to fail,
                        // since tx1 hasn't been broadcasted yet.
                        //
                        // Not exiting here will not be a problem when actually broadcasting,
                        // because for chains where `has_different_gas_calc`
                        // returns true, we await each transaction before
                        // broadcasting the next one.
                        if let Err(err) = estimate_gas(
                            tx,
                            &provider_info.provider,
                            self.args.gas_estimate_multiplier,
                            false,
                        )
                        .await
                        {
                            trace!("gas estimation failed: {err}");

                            // Restore gas value, since `estimate_gas` will remove it.
                            tx.set_gas_limit(gas);
                        }
                    }
                }

                let total_gas = total_gas_per_rpc.entry(tx_rpc.clone()).or_insert(0);
                *total_gas += tx.gas().expect("gas is set");
            }

            new_sequence.push_back(tx);
            // We only create a [`ScriptSequence`] object when we collect all the rpc related
            // transactions.
            if let Some(next_tx) = txes_iter.peek()
                && next_tx.rpc == tx_rpc
            {
                continue;
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

                let token_symbol = if self.script_config.evm_opts.networks.is_tempo() {
                    self.args.tempo.fee_token.map_or_else(
                        || "TIP-20".to_string(),
                        |fee_token| {
                            known_fee_token_symbol(fee_token)
                                .map(str::to_string)
                                .unwrap_or_else(|| fee_token.to_string())
                        },
                    )
                } else {
                    NamedChain::try_from(provider_info.chain)
                        .unwrap_or_default()
                        .native_currency_symbol()
                        .unwrap_or("ETH")
                        .to_string()
                };

                // We don't store it in the transactions, since we want the most updated value.
                // Right before broadcasting.
                //
                // Resolve the fees with the same overrides as the broadcast path so the
                // displayed values match what is sent. Skipped when `--with-gas-price` pins
                // the max fee directly.
                let resolved_eip1559_fees = if self.args.with_gas_price.is_none() {
                    if let Some(fees) = provider_info.eip1559_fees().copied() {
                        // `--batch` broadcasts via `broadcast_batch`, which applies no
                        // browser tip, so skip it here too. Best-effort.
                        let browser_suggested_tip =
                            if !self.args.batch && self.browser_wallet.is_some() {
                                provider_info.provider.get_max_priority_fee_per_gas().await.ok()
                            } else {
                                None
                            };
                        Some(resolve_broadcast_eip1559_fees(
                            fees,
                            None,
                            self.args.priority_gas_price.map(|p| p.to()),
                            browser_suggested_tip,
                        )?)
                    } else {
                        None
                    }
                } else {
                    None
                };

                // `per_gas` is the legacy gas price or, for EIP-1559, the `maxFeePerGas`
                // (a base-fee buffer plus the priority fee), which is what the transaction
                // can pay at most -- not the spot base fee shown by block explorers.
                let per_gas = if let Some(gas_price) = self.args.with_gas_price {
                    gas_price.to()
                } else if let Some(fees) = &resolved_eip1559_fees {
                    fees.max_fee_per_gas
                } else {
                    provider_info.gas_price()?
                };

                // Format a wei value as a trimmed gwei string.
                let fmt_gwei = |wei: u128| {
                    let raw = format_units(wei, 9)
                        .unwrap_or_else(|_| "[Could not calculate]".to_string());
                    raw.trim_end_matches('0').trim_end_matches('.').to_string()
                };

                let estimated_gas_price = fmt_gwei(per_gas);

                // (base fee, max priority fee) for the EIP-1559 breakdown.
                let fee_breakdown = resolved_eip1559_fees.as_ref().map(|fees| {
                    (fmt_gwei(fees.base_fee_per_gas), fmt_gwei(fees.max_priority_fee_per_gas))
                });

                let estimated_amount_raw = format_units(total_gas.saturating_mul(per_gas), 18)
                    .unwrap_or_else(|_| "[Could not calculate]".to_string());
                let estimated_amount = estimated_amount_raw.trim_end_matches('0');

                if shell::is_json() {
                    let mut json = serde_json::json!({
                        "chain": provider_info.chain,
                        "estimated_gas_price": estimated_gas_price,
                        "estimated_total_gas_used": total_gas,
                        "estimated_amount_required": estimated_amount,
                        "token_symbol": token_symbol,
                    });
                    if let Some((base_fee, priority_fee)) = &fee_breakdown {
                        json["estimated_max_fee_per_gas"] =
                            serde_json::Value::from(estimated_gas_price);
                        json["estimated_base_fee_per_gas"] =
                            serde_json::Value::from(base_fee.clone());
                        json["estimated_max_priority_fee_per_gas"] =
                            serde_json::Value::from(priority_fee.clone());
                    }
                    sh_println!("{}", json)?;
                } else {
                    sh_println!("\n==========================")?;
                    sh_println!("\nChain {}", provider_info.chain)?;

                    if let Some((base_fee, priority_fee)) = &fee_breakdown {
                        sh_println!("\nEstimated max fee per gas: {estimated_gas_price} gwei")?;
                        sh_println!("Estimated base fee per gas: {base_fee} gwei")?;
                        sh_println!("Estimated max priority fee per gas: {priority_fee} gwei")?;
                    } else {
                        sh_println!("\nEstimated gas price: {estimated_gas_price} gwei")?;
                    }
                    sh_println!("\nEstimated total gas used for script: {total_gas}")?;
                    sh_println!("\nEstimated amount required: {estimated_amount} {token_symbol}")?;
                    sh_println!("\n==========================")?;
                }
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
            browser_wallet: self.browser_wallet,
            build_data: self.build_data,
            sequence,
        })
    }

    /// Creates a [ScriptSequence] object from the given transactions.
    fn create_sequence(
        &self,
        multi: bool,
        chain: u64,
        transactions: VecDeque<TransactionWithMetadata<FEN::Network>>,
    ) -> Result<ScriptSequence<FEN::Network>> {
        // Paths are set to None for multi-chain sequences parts, because they don't need to be
        // saved to a separate file.
        let paths = if multi {
            None
        } else {
            Some(ScriptSequence::<FEN::Network>::get_paths(
                &self.script_config.config,
                &self.args.sig,
                &self.build_data.build_data.target,
                chain,
                !self.args.broadcast,
            )?)
        };

        let commit = get_commit_hash(&self.script_config.config.root);

        let local_addresses = match &self.build_data.predeploy_libraries {
            crate::build::ScriptPredeployLibraries::Default { local, .. }
            | crate::build::ScriptPredeployLibraries::Create2 { local, .. } => local.as_slice(),
        };
        let local_addresses = local_addresses
            .iter()
            .map(|library| library.address.to_checksum(None))
            .collect::<Vec<_>>();
        let libraries = self
            .build_data
            .libraries
            .libs
            .iter()
            .flat_map(|(file, libs)| {
                libs.iter()
                    .filter(|(_, address)| !local_addresses.contains(address))
                    .map(|(name, address)| format!("{}:{name}:{address}", file.to_string_lossy()))
            })
            .collect();

        let sequence = ScriptSequence {
            transactions,
            returns: self.execution_artifacts.returns.clone(),
            receipts: vec![],
            pending: vec![],
            paths,
            timestamp: now().as_millis(),
            libraries,
            chain,
            commit,
        };
        Ok(sequence)
    }
}
