use super::{
    multi::MultiChainSequence,
    providers::{ProviderInfo, ProvidersManager},
    sequence::ScriptSequence,
    *,
};
use crate::{
    cmd::{
        forge::script::{
            receipts::wait_for_receipts, transaction::TransactionWithMetadata, verify::VerifyBundle,
        },
        has_batch_support, has_different_gas_calc,
    },
    init_progress,
    opts::WalletType,
    update_progress,
};
use ethers::{
    prelude::{Provider, Signer, SignerMiddleware, TxHash},
    providers::{JsonRpcClient, Middleware},
    types::transaction::eip2718::TypedTransaction,
    utils::format_units,
};
use eyre::{ContextCompat, WrapErr};
use foundry_common::get_http_provider;
use foundry_config::Chain;
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::{
    cmp::min,
    collections::{hash_map::Entry, HashSet},
    fmt,
    sync::Arc,
};

impl ScriptArgs {
    /// Sends the transactions which haven't been broadcasted yet.
    pub async fn send_transactions(
        &self,
        deployment_sequence: &mut ScriptSequence,
        fork_url: &str,
        script_wallets: Vec<LocalWallet>,
    ) -> eyre::Result<()> {
        let provider = Arc::new(get_http_provider(fork_url));
        let already_broadcasted = deployment_sequence.receipts.len();

        if already_broadcasted < deployment_sequence.transactions.len() {
            let required_addresses = deployment_sequence
                .typed_transactions()
                .into_iter()
                .skip(already_broadcasted)
                .map(|(_, tx)| *tx.from().expect("No sender for onchain transaction!"))
                .collect();

            let local_wallets =
                self.wallets.find_all(provider.clone(), required_addresses, script_wallets).await?;
            let chain = local_wallets.values().last().wrap_err("Error accessing local wallet when trying to send onchain transaction, did you set a private key, mnemonic or keystore?")?.chain_id();

            // We only wait for a transaction receipt before sending the next transaction, if there
            // is more than one signer. There would be no way of assuring their order
            // otherwise. Or if the chain does not support batched transactions (eg. Arbitrum).
            let sequential_broadcast =
                local_wallets.len() != 1 || self.slow || !has_batch_support(chain);

            // Make a one-time gas price estimation
            let (gas_price, eip1559_fees) = {
                match deployment_sequence.transactions.front().unwrap().typed_tx() {
                    TypedTransaction::Legacy(_) | TypedTransaction::Eip2930(_) => {
                        (provider.get_gas_price().await.ok(), None)
                    }
                    TypedTransaction::Eip1559(_) => {
                        (None, provider.estimate_eip1559_fees(None).await.ok())
                    }
                }
            };

            // Iterate through transactions, matching the `from` field with the associated
            // wallet. Then send the transaction. Panics if we find a unknown `from`
            let sequence = deployment_sequence
                .typed_transactions()
                .into_iter()
                .skip(already_broadcasted)
                .map(|(_, tx)| {
                    let from = *tx.from().expect("No sender for onchain transaction!");
                    let signer = local_wallets.get(&from).expect("`find_all` returned incomplete.");

                    let mut tx = tx.clone();

                    tx.set_chain_id(chain);

                    if let Some(gas_price) = self.with_gas_price {
                        tx.set_gas_price(gas_price);
                    } else {
                        // fill gas price
                        match tx {
                            TypedTransaction::Eip2930(_) | TypedTransaction::Legacy(_) => {
                                tx.set_gas_price(gas_price.expect("Could not get gas_price."));
                            }
                            TypedTransaction::Eip1559(ref mut inner) => {
                                let eip1559_fees =
                                    eip1559_fees.expect("Could not get eip1559 fee estimation.");
                                inner.max_fee_per_gas = Some(eip1559_fees.0);
                                inner.max_priority_fee_per_gas = Some(eip1559_fees.1);
                            }
                        }
                    }

                    (tx, signer)
                })
                .collect::<Vec<_>>();

            let pb = init_progress!(deployment_sequence.transactions, "txes");

            // We send transactions and wait for receipts in batches of 100, since some networks
            // cannot handle more than that.
            let batch_size = 100;
            let mut index = 0;

            for (batch_number, batch) in sequence.chunks(batch_size).map(|f| f.to_vec()).enumerate()
            {
                let mut pending_transactions = vec![];

                println!(
                    "##\nSending transactions [{} - {}].",
                    batch_number * batch_size,
                    batch_number * batch_size + min(batch_size, batch.len()) - 1
                );
                for (tx, signer) in batch.into_iter() {
                    let tx_hash = self.send_transaction(tx, signer, sequential_broadcast, fork_url);

                    if sequential_broadcast {
                        let tx_hash = tx_hash.await?;
                        deployment_sequence.add_pending(index, tx_hash);

                        update_progress!(pb, (index + already_broadcasted));
                        index += 1;

                        wait_for_receipts(vec![tx_hash], deployment_sequence, provider.clone())
                            .await?;
                    } else {
                        pending_transactions.push(tx_hash);
                    }
                }

                if !pending_transactions.is_empty() {
                    let mut buffer = futures::stream::iter(pending_transactions).buffered(7);

                    let mut tx_hashes = vec![];

                    while let Some(tx_hash) = buffer.next().await {
                        let tx_hash = tx_hash?;
                        deployment_sequence.add_pending(index, tx_hash);
                        tx_hashes.push(tx_hash);

                        update_progress!(pb, (index + already_broadcasted));
                        index += 1;
                    }

                    // Checkpoint save
                    deployment_sequence.save()?;

                    if !sequential_broadcast {
                        println!("##\nWaiting for receipts.");
                        wait_for_receipts(tx_hashes, deployment_sequence, provider.clone()).await?;
                    }
                }

                // Checkpoint save
                deployment_sequence.save()?;
            }
        }

        println!("\n\n==========================");
        println!("\nONCHAIN EXECUTION COMPLETE & SUCCESSFUL.");
        Ok(())
    }

    pub async fn send_transaction(
        &self,
        tx: TypedTransaction,
        signer: &WalletType,
        sequential_broadcast: bool,
        fork_url: &str,
    ) -> Result<TxHash, BroadcastError> {
        let from = tx.from().expect("no sender");

        if sequential_broadcast {
            let nonce = foundry_utils::next_nonce(*from, fork_url, None).await.map_err(|_| {
                BroadcastError::Simple("Not able to query the EOA nonce.".to_string())
            })?;

            let tx_nonce = tx.nonce().expect("no nonce");

            if nonce != *tx_nonce {
                return Err(BroadcastError::Simple(
                    "EOA nonce changed unexpectedly while sending transactions.".to_string(),
                ))
            }
        }

        match signer {
            WalletType::Local(signer) => self.broadcast(signer, tx).await,
            WalletType::Ledger(signer) => self.broadcast(signer, tx).await,
            WalletType::Trezor(signer) => self.broadcast(signer, tx).await,
        }
    }

    /// Executes the passed transactions in sequence, and if no error has occurred, broadcasts
    /// them.
    pub async fn handle_broadcastable_transactions(
        &self,
        target: &ArtifactId,
        result: ScriptResult,
        libraries: Libraries,
        decoder: &mut CallTraceDecoder,
        mut script_config: ScriptConfig,
        verify: VerifyBundle,
    ) -> eyre::Result<()> {
        if let Some(txs) = result.transactions {
            let num_fork_rpcs = txs.iter().filter(|tx| tx.rpc.is_some()).count();
            let total_rpcs = num_fork_rpcs + script_config.evm_opts.fork_url.is_some() as usize;

            if total_rpcs > 0 {
                self.check_multi_chain_constraints(total_rpcs, &libraries)?;

                let gas_filled_txs = if self.skip_simulation {
                    println!("\nSKIPPING ON CHAIN SIMULATION.");
                    txs.into_iter()
                        .map(|tx| TransactionWithMetadata::from_typed_transaction(tx.transaction))
                        .collect()
                } else {
                    self.execute_transactions(
                        txs,
                        &mut script_config,
                        decoder,
                        &verify.known_contracts,
                    )
                    .await
                    .map_err(|err| {
                        eyre::eyre!(
                            "{err}\n\nTransaction failed when running the on-chain simulation. Check the trace above for more information."
                        )
                    })?
                };

                let returns = self.get_returns(&script_config, &result.returned)?;
                let mut deployments = self
                    .handle_chain_requirements(
                        gas_filled_txs,
                        target,
                        &mut script_config.config,
                        returns,
                    )
                    .await?;

                if deployments.len() > 1 {
                    let multi = MultiChainSequence::new(
                        deployments.clone(),
                        &self.sig,
                        target,
                        &script_config.config.broadcast,
                        self.broadcast,
                    )?;

                    if self.broadcast {
                        self.multi_chain_deployment(
                            multi,
                            libraries,
                            &script_config.config,
                            verify,
                        )
                        .await?;
                    }
                } else if self.broadcast {
                    let mut deployment_sequence = deployments.first_mut().expect("to be set.");
                    let rpc = deployment_sequence
                        .transactions
                        .front()
                        .as_ref()
                        .expect("to be set")
                        .rpc
                        .clone()
                        .expect("to be set.");

                    deployment_sequence.add_libraries(libraries);

                    self.send_transactions(&mut deployment_sequence, &rpc, result.script_wallets)
                        .await?;

                    if self.verify {
                        deployment_sequence.verify_contracts(&script_config.config, verify).await?;
                    }
                }

                if !self.broadcast {
                    println!("\nSIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.");
                }
            } else {
                println!("\nIf you wish to simulate on-chain transactions pass a RPC URL.");
            }
        } else if self.broadcast {
            eyre::bail!("No onchain transactions generated in script");
        }
        Ok(())
    }

    /// Certain features are disabled for multi chain deployments, and if tried, will return
    /// error.
    fn check_multi_chain_constraints(
        &self,
        total_rpcs: usize,
        libraries: &Libraries,
    ) -> eyre::Result<()> {
        if total_rpcs > 1 {
            eprintln!(
                "{}",
                Paint::yellow(
                    "Multi chain deployment is still under development. Use with caution."
                )
            );
            if !libraries.libs.is_empty() {
                eyre::bail!(
                    "Multi chain deployment does not support library linking at the moment."
                )
            }
            if self.skip_simulation {
                eyre::bail!(
                    "Multi chain deployment does not support skipping simulations at the moment."
                );
            }
            if self.verify {
                eyre::bail!("Multi chain deployment does not contract verification at the moment.");
            }
        }
        Ok(())
    }

    /// Modify each transaction according to the specific chain requirements (transaction type
    /// and/or gas calculations).
    async fn handle_chain_requirements(
        &self,
        transactions: VecDeque<TransactionWithMetadata>,
        target: &ArtifactId,
        config: &mut Config,
        returns: HashMap<String, NestedValue>,
    ) -> eyre::Result<Vec<ScriptSequence>> {
        let arg_url = self.evm_opts.fork_url.clone().unwrap_or_default();

        let last_rpc = &transactions.back().expect("exists; qed").rpc;
        let is_multi_deployment = transactions.iter().any(|tx| &tx.rpc != last_rpc);

        let mut total_gas_per_rpc: HashMap<String, (U256, bool)> = HashMap::new();

        // Required to find user provided wallets.
        let mut addresses = HashSet::new();
        // Batches sequences from different rpcs.
        let mut new_txes = VecDeque::new();
        let mut manager = ProvidersManager::default();
        let mut deployments = vec![];

        let mut txes_iter = transactions.into_iter().peekable();

        while let Some(mut tx) = txes_iter.next() {
            let tx_rpc = tx.rpc.unwrap_or_else(|| arg_url.clone());

            if tx_rpc.is_empty() {
                eyre::bail!("Transaction needs an associated RPC if it is to be broadcasted.");
            }

            tx.rpc = Some(tx_rpc.clone());

            let provider_info = match manager.inner.entry(tx_rpc.clone()) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let info = ProviderInfo::new(&tx_rpc, &tx, self.slow).await?;
                    entry.insert(info)
                }
            };

            let mut is_legacy = self.legacy;
            if let Chain::Named(chain) = Chain::from(provider_info.chain) {
                is_legacy |= chain.is_legacy();
            };

            tx.change_type(is_legacy);

            if !self.skip_simulation {
                let typed_tx = tx.typed_tx_mut();

                if has_different_gas_calc(provider_info.chain) {
                    self.estimate_gas(typed_tx, &provider_info.provider).await?;
                }

                let (total_gas, _) =
                    total_gas_per_rpc.entry(tx_rpc.clone()).or_insert((U256::zero(), !is_legacy));

                *total_gas += *typed_tx.gas().expect("gas is set");
            }

            let from = tx.typed_tx().from().expect("No sender for onchain transaction.");
            if !provider_info.wallets.contains_key(from) {
                addresses.insert(*from);
            }

            if let Some(next_tx) = txes_iter.peek() {
                if next_tx.rpc == tx.rpc {
                    new_txes.push_back(tx);
                    continue
                }
            }

            new_txes.push_back(tx);

            if self.broadcast {
                provider_info.wallets.extend(
                    self.wallets
                        .find_all(provider_info.provider.clone(), addresses.clone(), vec![])
                        .await?,
                );
                provider_info.sequential &= provider_info.wallets.len() != 1;
            }

            addresses.clear();
            let sequence = ScriptSequence::new(
                new_txes,
                returns.clone(),
                &self.sig,
                target,
                config,
                provider_info.chain,
                self.broadcast,
                is_multi_deployment,
            )?;

            deployments.push(sequence);

            new_txes = VecDeque::new();
        }

        if !self.skip_simulation {
            for (rpc, (total_gas, is_eip1559)) in total_gas_per_rpc {
                let provider_info = manager.inner.get(&rpc).expect("to be set.");

                // We don't store it in the transactions, since we want the most updated value.
                // Right before broadcasting.
                let per_gas = if let Some(gas_price) = self.with_gas_price {
                    gas_price
                } else if is_eip1559 {
                    provider_info.provider.estimate_eip1559_fees(None).await?.0
                } else {
                    provider_info.provider.get_gas_price().await?
                };

                println!("\n==========================");
                println!("\nChain {}", provider_info.chain);
                println!("\nEstimated total gas used for script: {}", total_gas);
                println!(
                    "\nEstimated amount required: {} ETH",
                    format_units(total_gas.saturating_mul(per_gas), 18)
                        .unwrap_or_else(|_| "[Could not calculate]".to_string())
                        .trim_end_matches('0')
                );
                println!("\n==========================");
            }
        }
        Ok(deployments)
    }

    /// Uses the signer to submit a transaction to the network. If it fails, it tries to
    /// retrieve the transaction hash that can be used on a later run with `--resume`.
    async fn broadcast<T, U>(
        &self,
        signer: &SignerMiddleware<T, U>,
        mut legacy_or_1559: TypedTransaction,
    ) -> Result<TxHash, BroadcastError>
    where
        T: Middleware,
        U: Signer,
    {
        tracing::debug!("sending transaction: {:?}", legacy_or_1559);

        // Chains which use `eth_estimateGas` are being sent sequentially and require their gas
        // to be re-estimated right before broadcasting.
        if has_different_gas_calc(signer.signer().chain_id()) || self.skip_simulation {
            // if already set, some RPC endpoints might simply return the gas value that is
            // already set in the request and omit the estimate altogether, so
            // we remove it here
            let _ = legacy_or_1559.gas_mut().take();

            self.estimate_gas(&mut legacy_or_1559, signer.provider()).await?;
        }

        // Signing manually so we skip `fill_transaction` and its `eth_createAccessList`
        // request.
        let signature = signer
            .sign_transaction(
                &legacy_or_1559,
                *legacy_or_1559.from().expect("Tx should have a `from`."),
            )
            .await
            .map_err(|err| BroadcastError::Simple(err.to_string()))?;

        // Submit the raw transaction
        let pending = signer
            .provider()
            .send_raw_transaction(legacy_or_1559.rlp_signed(&signature))
            .await
            .map_err(|err| BroadcastError::Simple(err.to_string()))?;

        Ok(pending.tx_hash())
    }

    async fn estimate_gas<T>(
        &self,
        tx: &mut TypedTransaction,
        provider: &Provider<T>,
    ) -> Result<(), BroadcastError>
    where
        T: JsonRpcClient,
    {
        tx.set_gas(
            provider
                .estimate_gas(tx, None)
                .await
                .wrap_err_with(|| format!("Failed to estimate gas for tx: {}", tx.sighash()))
                .map_err(|err| BroadcastError::Simple(err.to_string()))? *
                self.gas_estimate_multiplier /
                100,
        );
        Ok(())
    }
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum BroadcastError {
    Simple(String),
    ErrorWithTxHash(String, TxHash),
}

impl fmt::Display for BroadcastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BroadcastError::Simple(err) => write!(f, "{err}"),
            BroadcastError::ErrorWithTxHash(err, tx_hash) => {
                write!(f, "\nFailed to wait for transaction {tx_hash:?}:\n{err}")
            }
        }
    }
}
