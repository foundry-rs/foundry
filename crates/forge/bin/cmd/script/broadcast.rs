use super::{
    artifacts::ArtifactInfo,
    build::LinkedBuildData,
    executor::{ExecutionArtifacts, ExecutionData, PreSimulationState},
    multi::MultiChainSequence,
    providers::ProvidersManager,
    receipts::clear_pendings,
    runner::ScriptRunner,
    sequence::ScriptSequence,
    transaction::TransactionWithMetadata,
    verify::VerifyBundle,
    ScriptArgs, ScriptConfig,
};
use crate::cmd::script::transaction::AdditionalContract;
use alloy_primitives::{utils::format_units, Address, TxHash, U256};
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_providers::{JsonRpcClient, Middleware, Provider};
use ethers_signers::Signer;
use eyre::{bail, Context, ContextCompat, Result};
use forge::{inspectors::cheatcodes::BroadcastableTransactions, traces::render_trace_arena};
use foundry_cli::{
    init_progress, update_progress,
    utils::{has_batch_support, has_different_gas_calc},
};
use foundry_common::{
    get_contract_name,
    provider::{
        alloy::RpcUrl,
        ethers::{estimate_eip1559_fees, try_get_http_provider, RetryProvider},
    },
    shell,
    types::{ToAlloy, ToEthers},
};
use foundry_compilers::artifacts::Libraries;
use foundry_config::Config;
use foundry_wallets::WalletSigner;
use futures::{future::join_all, StreamExt};
use parking_lot::RwLock;
use std::{
    cmp::min,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::Arc,
};

impl PreSimulationState {
    pub async fn fill_metadata(self) -> Result<FilledTransactionsState> {
        let transactions = if let Some(txs) = self.execution_result.transactions.as_ref() {
            if self.args.skip_simulation {
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
            build_data: self.build_data,
            execution_data: self.execution_data,
            execution_artifacts: self.execution_artifacts,
            transactions,
        })
    }

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

        if self.script_config.evm_opts.verbosity > 3 {
            println!("==========================");
            println!("Simulated On-chain Traces:\n");
        }

        let contracts = self.build_data.get_flattened_contracts(false);
        let address_to_abi: BTreeMap<Address, ArtifactInfo> = self
            .execution_artifacts
            .decoder
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
                        let gas =
                            U256::from(result.gas_used * self.args.gas_estimate_multiplier / 100);
                        tx.gas = Some(gas);
                    }
                }

                let tx = TransactionWithMetadata::new(
                    tx,
                    transaction.rpc,
                    &result,
                    &address_to_abi,
                    &self.execution_artifacts.decoder,
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
            if tx.is_none() || self.script_config.evm_opts.verbosity > 3 {
                // Identify all contracts created during the call.
                if traces.is_empty() {
                    eyre::bail!(
                        "forge script requires tracing enabled to collect created contracts"
                    );
                }

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

    /// Build the multiple runners from different forks.
    async fn build_runners(&self) -> Result<HashMap<RpcUrl, ScriptRunner>> {
        if !shell::verbosity().is_silent() {
            let n = self.script_config.total_rpcs.len();
            let s = if n != 1 { "s" } else { "" };
            println!("\n## Setting up {n} EVM{s}.");
        }

        let futs = self
            .script_config
            .total_rpcs
            .iter()
            .map(|rpc| async {
                let mut script_config = self.script_config.clone();
                let runner = script_config.get_runner(Some(rpc.clone()), false).await?;
                Ok((rpc.clone(), runner))
            })
            .collect::<Vec<_>>();

        join_all(futs).await.into_iter().collect()
    }

    fn no_simulation(
        &self,
        transactions: BroadcastableTransactions,
    ) -> Result<VecDeque<TransactionWithMetadata>> {
        Ok(transactions
            .into_iter()
            .map(|tx| TransactionWithMetadata::from_tx_request(tx.transaction))
            .collect())
    }
}

pub struct FilledTransactionsState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_artifacts: ExecutionArtifacts,
    pub transactions: VecDeque<TransactionWithMetadata>,
}

impl FilledTransactionsState {
    /// Returns all transactions of the [`TransactionWithMetadata`] type in a list of
    /// [`ScriptSequence`]. List length will be higher than 1, if we're dealing with a multi
    /// chain deployment.
    ///
    /// Each transaction will be added with the correct transaction type and gas estimation.
    pub async fn bundle(self) -> Result<BundledState> {
        // User might be using both "in-code" forks and `--fork-url`.
        let last_rpc = &self.transactions.back().expect("exists; qed").rpc;
        let is_multi_deployment = self.transactions.iter().any(|tx| &tx.rpc != last_rpc);

        let mut total_gas_per_rpc: HashMap<RpcUrl, U256> = HashMap::new();

        // Batches sequence of transactions from different rpcs.
        let mut new_sequence = VecDeque::new();
        let mut manager = ProvidersManager::default();
        let mut sequences = vec![];

        // Peeking is used to check if the next rpc url is different. If so, it creates a
        // [`ScriptSequence`] from all the collected transactions up to this point.
        let mut txes_iter = self.transactions.clone().into_iter().peekable();

        while let Some(mut tx) = txes_iter.next() {
            let tx_rpc = match tx.rpc.clone() {
                Some(rpc) => rpc,
                None => {
                    let rpc = self.args.evm_opts.ensure_fork_url()?.clone();
                    // Fills the RPC inside the transaction, if missing one.
                    tx.rpc = Some(rpc.clone());
                    rpc
                }
            };

            let provider_info = manager.get_or_init_provider(&tx_rpc, self.args.legacy).await?;

            // Handles chain specific requirements.
            tx.change_type(provider_info.is_legacy);
            tx.transaction.set_chain_id(provider_info.chain);

            if !self.args.skip_simulation {
                let typed_tx = tx.typed_tx_mut();

                if has_different_gas_calc(provider_info.chain) {
                    trace!("estimating with different gas calculation");
                    let gas = *typed_tx.gas().expect("gas is set by simulation.");

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
                    if let Err(err) = self.estimate_gas(typed_tx, &provider_info.provider).await {
                        trace!("gas estimation failed: {err}");

                        // Restore gas value, since `estimate_gas` will remove it.
                        typed_tx.set_gas(gas);
                    }
                }

                let total_gas = total_gas_per_rpc.entry(tx_rpc.clone()).or_insert(U256::ZERO);
                *total_gas += (*typed_tx.gas().expect("gas is set")).to_alloy();
            }

            new_sequence.push_back(tx);
            // We only create a [`ScriptSequence`] object when we collect all the rpc related
            // transactions.
            if let Some(next_tx) = txes_iter.peek() {
                if next_tx.rpc == Some(tx_rpc) {
                    continue;
                }
            }

            let sequence = ScriptSequence::new(
                new_sequence,
                self.execution_artifacts.returns.clone(),
                &self.args.sig,
                &self.build_data.build_data.target,
                provider_info.chain.into(),
                &self.script_config.config,
                self.args.broadcast,
                is_multi_deployment,
            )?;

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
                    gas_price
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
        Ok(BundledState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            execution_data: self.execution_data,
            execution_artifacts: self.execution_artifacts,
            sequences,
        })
    }

    async fn estimate_gas<T>(&self, tx: &mut TypedTransaction, provider: &Provider<T>) -> Result<()>
    where
        T: JsonRpcClient,
    {
        // if already set, some RPC endpoints might simply return the gas value that is already
        // set in the request and omit the estimate altogether, so we remove it here
        let _ = tx.gas_mut().take();

        tx.set_gas(
            provider
                .estimate_gas(tx, None)
                .await
                .wrap_err_with(|| format!("Failed to estimate gas for tx: {:?}", tx.sighash()))? *
                self.args.gas_estimate_multiplier /
                100,
        );
        Ok(())
    }
}

pub struct BundledState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_artifacts: ExecutionArtifacts,
    pub sequences: Vec<ScriptSequence>,
}

impl ScriptArgs {
    /// Sends the transactions which haven't been broadcasted yet.
    pub async fn send_transactions(
        &self,
        deployment_sequence: &mut ScriptSequence,
        fork_url: &str,
        signers: &HashMap<Address, WalletSigner>,
    ) -> Result<()> {
        let provider = Arc::new(try_get_http_provider(fork_url)?);
        let already_broadcasted = deployment_sequence.receipts.len();

        if already_broadcasted < deployment_sequence.transactions.len() {
            let required_addresses: HashSet<Address> = deployment_sequence
                .typed_transactions()
                .skip(already_broadcasted)
                .map(|tx| (*tx.from().expect("No sender for onchain transaction!")).to_alloy())
                .collect();

            let (send_kind, chain) = if self.unlocked {
                let chain = provider.get_chainid().await?;
                let mut senders = HashSet::from([self
                    .evm_opts
                    .sender
                    .wrap_err("--sender must be set with --unlocked")?]);
                // also take all additional senders that where set manually via broadcast
                senders.extend(
                    deployment_sequence
                        .typed_transactions()
                        .filter_map(|tx| tx.from().copied().map(|addr| addr.to_alloy())),
                );
                (SendTransactionsKind::Unlocked(senders), chain.as_u64())
            } else {
                let mut missing_addresses = Vec::new();

                println!("\n###\nFinding wallets for all the necessary addresses...");
                for addr in &required_addresses {
                    if !signers.contains_key(addr) {
                        missing_addresses.push(addr);
                    }
                }

                if !missing_addresses.is_empty() {
                    let mut error_msg = String::new();

                    // This is an actual used address
                    if required_addresses.contains(&Config::DEFAULT_SENDER) {
                        error_msg += "\nYou seem to be using Foundry's default sender. Be sure to set your own --sender.\n";
                    }

                    eyre::bail!(
                        "{}No associated wallet for addresses: {:?}. Unlocked wallets: {:?}",
                        error_msg,
                        missing_addresses,
                        signers.keys().collect::<Vec<_>>()
                    );
                }

                let chain = provider.get_chainid().await?.as_u64();

                (SendTransactionsKind::Raw(signers), chain)
            };

            // We only wait for a transaction receipt before sending the next transaction, if there
            // is more than one signer. There would be no way of assuring their order
            // otherwise. Or if the chain does not support batched transactions (eg. Arbitrum).
            let sequential_broadcast =
                send_kind.signers_count() != 1 || self.slow || !has_batch_support(chain);

            // Make a one-time gas price estimation
            let (gas_price, eip1559_fees) = {
                match deployment_sequence.transactions.front().unwrap().typed_tx() {
                    TypedTransaction::Eip1559(_) => {
                        let fees = estimate_eip1559_fees(&provider, Some(chain))
                            .await
                            .wrap_err("Failed to estimate EIP1559 fees. This chain might not support EIP1559, try adding --legacy to your command.")?;

                        (None, Some(fees))
                    }
                    _ => (provider.get_gas_price().await.ok(), None),
                }
            };

            // Iterate through transactions, matching the `from` field with the associated
            // wallet. Then send the transaction. Panics if we find a unknown `from`
            let sequence = deployment_sequence
                .transactions
                .iter()
                .skip(already_broadcasted)
                .map(|tx_with_metadata| {
                    let tx = tx_with_metadata.typed_tx();
                    let from = (*tx.from().expect("No sender for onchain transaction!")).to_alloy();

                    let kind = send_kind.for_sender(&from)?;
                    let is_fixed_gas_limit = tx_with_metadata.is_fixed_gas_limit;

                    let mut tx = tx.clone();

                    tx.set_chain_id(chain);

                    if let Some(gas_price) = self.with_gas_price {
                        tx.set_gas_price(gas_price.to_ethers());
                    } else {
                        // fill gas price
                        match tx {
                            TypedTransaction::Eip1559(ref mut inner) => {
                                let eip1559_fees =
                                    eip1559_fees.expect("Could not get eip1559 fee estimation.");
                                if let Some(priority_gas_price) = self.priority_gas_price {
                                    inner.max_priority_fee_per_gas =
                                        Some(priority_gas_price.to_ethers());
                                } else {
                                    inner.max_priority_fee_per_gas = Some(eip1559_fees.1);
                                }
                                inner.max_fee_per_gas = Some(eip1559_fees.0);
                            }
                            _ => {
                                tx.set_gas_price(gas_price.expect("Could not get gas_price."));
                            }
                        }
                    }

                    Ok((tx, kind, is_fixed_gas_limit))
                })
                .collect::<Result<Vec<_>>>()?;

            let pb = init_progress!(deployment_sequence.transactions, "txes");

            // We send transactions and wait for receipts in batches of 100, since some networks
            // cannot handle more than that.
            let batch_size = 100;
            let mut index = 0;

            for (batch_number, batch) in sequence.chunks(batch_size).map(|f| f.to_vec()).enumerate()
            {
                let mut pending_transactions = vec![];

                shell::println(format!(
                    "##\nSending transactions [{} - {}].",
                    batch_number * batch_size,
                    batch_number * batch_size + min(batch_size, batch.len()) - 1
                ))?;
                for (tx, kind, is_fixed_gas_limit) in batch.into_iter() {
                    let tx_hash = self.send_transaction(
                        provider.clone(),
                        tx,
                        kind,
                        sequential_broadcast,
                        fork_url,
                        is_fixed_gas_limit,
                    );

                    if sequential_broadcast {
                        let tx_hash = tx_hash.await?;
                        deployment_sequence.add_pending(index, tx_hash);

                        update_progress!(pb, (index + already_broadcasted));
                        index += 1;

                        clear_pendings(provider.clone(), deployment_sequence, Some(vec![tx_hash]))
                            .await?;
                    } else {
                        pending_transactions.push(tx_hash);
                    }
                }

                if !pending_transactions.is_empty() {
                    let mut buffer = futures::stream::iter(pending_transactions).buffered(7);

                    while let Some(tx_hash) = buffer.next().await {
                        let tx_hash = tx_hash?;
                        deployment_sequence.add_pending(index, tx_hash);

                        update_progress!(pb, (index + already_broadcasted));
                        index += 1;
                    }

                    // Checkpoint save
                    deployment_sequence.save()?;

                    if !sequential_broadcast {
                        shell::println("##\nWaiting for receipts.")?;
                        clear_pendings(provider.clone(), deployment_sequence, None).await?;
                    }
                }

                // Checkpoint save
                deployment_sequence.save()?;
            }
        }

        shell::println("\n\n==========================")?;
        shell::println("\nONCHAIN EXECUTION COMPLETE & SUCCESSFUL.")?;

        let (total_gas, total_gas_price, total_paid) = deployment_sequence.receipts.iter().fold(
            (U256::ZERO, U256::ZERO, U256::ZERO),
            |acc, receipt| {
                let gas_used = receipt.gas_used.unwrap_or_default().to_alloy();
                let gas_price = receipt.effective_gas_price.unwrap_or_default().to_alloy();
                (acc.0 + gas_used, acc.1 + gas_price, acc.2 + gas_used * gas_price)
            },
        );
        let paid = format_units(total_paid, 18).unwrap_or_else(|_| "N/A".to_string());
        let avg_gas_price =
            format_units(total_gas_price / U256::from(deployment_sequence.receipts.len()), 9)
                .unwrap_or_else(|_| "N/A".to_string());
        shell::println(format!(
            "Total Paid: {} ETH ({} gas * avg {} gwei)",
            paid.trim_end_matches('0'),
            total_gas,
            avg_gas_price.trim_end_matches('0').trim_end_matches('.')
        ))?;

        Ok(())
    }

    async fn send_transaction(
        &self,
        provider: Arc<RetryProvider>,
        mut tx: TypedTransaction,
        kind: SendTransactionKind<'_>,
        sequential_broadcast: bool,
        fork_url: &str,
        is_fixed_gas_limit: bool,
    ) -> Result<TxHash> {
        let from = tx.from().expect("no sender");

        if sequential_broadcast {
            let nonce = forge::next_nonce((*from).to_alloy(), fork_url, None)
                .await
                .map_err(|_| eyre::eyre!("Not able to query the EOA nonce."))?;

            let tx_nonce = tx.nonce().expect("no nonce");
            if let Ok(tx_nonce) = u64::try_from(tx_nonce.to_alloy()) {
                if nonce != tx_nonce {
                    bail!("EOA nonce changed unexpectedly while sending transactions. Expected {tx_nonce} got {nonce} from provider.")
                }
            }
        }

        match kind {
            SendTransactionKind::Unlocked(addr) => {
                debug!("sending transaction from unlocked account {:?}: {:?}", addr, tx);

                // Chains which use `eth_estimateGas` are being sent sequentially and require their
                // gas to be re-estimated right before broadcasting.
                if !is_fixed_gas_limit &&
                    (has_different_gas_calc(provider.get_chainid().await?.as_u64()) ||
                        self.skip_simulation)
                {
                    self.estimate_gas(&mut tx, &provider).await?;
                }

                // Submit the transaction
                let pending = provider.send_transaction(tx, None).await?;

                Ok(pending.tx_hash().to_alloy())
            }
            SendTransactionKind::Raw(signer) => self.broadcast(provider, signer, tx).await,
        }
    }

    /// Executes the created transactions, and if no error has occurred, broadcasts
    /// them.
    pub async fn handle_broadcastable_transactions(
        &self,
        mut state: BundledState,
        verify: VerifyBundle,
    ) -> Result<()> {
        if state.script_config.has_multiple_rpcs() {
            trace!(target: "script", "broadcasting multi chain deployment");

            let multi = MultiChainSequence::new(
                state.sequences.clone(),
                &self.sig,
                &state.build_data.build_data.target,
                &state.script_config.config,
                self.broadcast,
            )?;

            if self.broadcast {
                self.multi_chain_deployment(
                    multi,
                    state.build_data.libraries,
                    &state.script_config.config,
                    verify,
                    &state.script_config.script_wallets.into_multi_wallet().into_signers()?,
                )
                .await?;
            }
        } else if self.broadcast {
            self.single_deployment(
                state.sequences.first_mut().expect("missing deployment"),
                state.script_config,
                state.build_data.libraries,
                verify,
            )
            .await?;
        }

        if !self.broadcast {
            shell::println("\nSIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.")?;
        }
        Ok(())
    }

    /// Broadcasts a single chain script.
    async fn single_deployment(
        &self,
        deployment_sequence: &mut ScriptSequence,
        script_config: ScriptConfig,
        libraries: Libraries,
        verify: VerifyBundle,
    ) -> Result<()> {
        trace!(target: "script", "broadcasting single chain deployment");

        if self.verify {
            deployment_sequence.verify_preflight_check(&script_config.config, &verify)?;
        }

        let rpc = script_config.total_rpcs.into_iter().next().expect("exists; qed");

        deployment_sequence.add_libraries(libraries);

        let signers = script_config.script_wallets.into_multi_wallet().into_signers()?;

        self.send_transactions(deployment_sequence, &rpc, &signers).await?;

        if self.verify {
            return deployment_sequence.verify_contracts(&script_config.config, verify).await;
        }
        Ok(())
    }

    /// Uses the signer to submit a transaction to the network. If it fails, it tries to retrieve
    /// the transaction hash that can be used on a later run with `--resume`.
    async fn broadcast(
        &self,
        provider: Arc<RetryProvider>,
        signer: &WalletSigner,
        mut legacy_or_1559: TypedTransaction,
    ) -> Result<TxHash> {
        debug!("sending transaction: {:?}", legacy_or_1559);

        // Chains which use `eth_estimateGas` are being sent sequentially and require their gas
        // to be re-estimated right before broadcasting.
        if has_different_gas_calc(signer.chain_id()) || self.skip_simulation {
            // if already set, some RPC endpoints might simply return the gas value that is
            // already set in the request and omit the estimate altogether, so
            // we remove it here
            let _ = legacy_or_1559.gas_mut().take();

            self.estimate_gas(&mut legacy_or_1559, &provider).await?;
        }

        // Signing manually so we skip `fill_transaction` and its `eth_createAccessList`
        // request.
        let signature = signer
            .sign_transaction(&legacy_or_1559)
            .await
            .wrap_err("Failed to sign transaction")?;

        // Submit the raw transaction
        let pending = provider.send_raw_transaction(legacy_or_1559.rlp_signed(&signature)).await?;

        Ok(pending.tx_hash().to_alloy())
    }

    async fn estimate_gas<T>(&self, tx: &mut TypedTransaction, provider: &Provider<T>) -> Result<()>
    where
        T: JsonRpcClient,
    {
        // if already set, some RPC endpoints might simply return the gas value that is already
        // set in the request and omit the estimate altogether, so we remove it here
        let _ = tx.gas_mut().take();

        tx.set_gas(
            provider
                .estimate_gas(tx, None)
                .await
                .wrap_err_with(|| format!("Failed to estimate gas for tx: {:?}", tx.sighash()))? *
                self.gas_estimate_multiplier /
                100,
        );
        Ok(())
    }
}

/// How to send a single transaction
#[derive(Clone)]
enum SendTransactionKind<'a> {
    Unlocked(Address),
    Raw(&'a WalletSigner),
}

/// Represents how to send _all_ transactions
enum SendTransactionsKind<'a> {
    /// Send via `eth_sendTransaction` and rely on the  `from` address being unlocked.
    Unlocked(HashSet<Address>),
    /// Send a signed transaction via `eth_sendRawTransaction`
    Raw(&'a HashMap<Address, WalletSigner>),
}

impl SendTransactionsKind<'_> {
    /// Returns the [`SendTransactionKind`] for the given address
    ///
    /// Returns an error if no matching signer is found or the address is not unlocked
    fn for_sender(&self, addr: &Address) -> Result<SendTransactionKind<'_>> {
        match self {
            SendTransactionsKind::Unlocked(unlocked) => {
                if !unlocked.contains(addr) {
                    bail!("Sender address {:?} is not unlocked", addr)
                }
                Ok(SendTransactionKind::Unlocked(*addr))
            }
            SendTransactionsKind::Raw(wallets) => {
                if let Some(wallet) = wallets.get(addr) {
                    Ok(SendTransactionKind::Raw(wallet))
                } else {
                    bail!("No matching signer for {:?} found", addr)
                }
            }
        }
    }

    /// How many signers are set
    fn signers_count(&self) -> usize {
        match self {
            SendTransactionsKind::Unlocked(addr) => addr.len(),
            SendTransactionsKind::Raw(signers) => signers.len(),
        }
    }
}
