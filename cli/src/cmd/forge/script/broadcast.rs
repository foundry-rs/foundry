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
    utils::format_units,
};
use eyre::{ContextCompat, WrapErr};
use foundry_common::get_http_provider;
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::{cmp::min, collections::hash_map::Entry, fmt, ops::Mul, sync::Arc};

impl ScriptArgs {
    /// Sends the transactions which haven't been broadcasted yet.
    pub async fn send_transactions(
        &self,
        deployment_sequence: &mut ScriptSequence,
        fork_url: &str,
        script_wallets: &[LocalWallet],
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
                    let signer = local_wallets
                        .get(&from)
                        .wrap_err("`wallets.find_all` returned incomplete.")?;

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

                    Ok((tx, signer))
                })
                .collect::<eyre::Result<Vec<_>>>()?;

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

        let (total_gas, total_gas_price, total_paid) = deployment_sequence.receipts.iter().fold(
            (U256::zero(), U256::zero(), U256::zero()),
            |acc, receipt| {
                let gas_used = receipt.gas_used.unwrap_or_default();
                let gas_price = receipt.effective_gas_price.unwrap_or_default();
                (acc.0 + gas_used, acc.1 + gas_price, acc.2 + gas_used.mul(gas_price))
            },
        );
        let paid = format_units(total_paid, 18).unwrap_or_else(|_| "N/A".into());
        let avg_gas_price = format_units(total_gas_price / deployment_sequence.receipts.len(), 9)
            .unwrap_or_else(|_| "N/A".into());
        println!(
            "Total Paid: {} ETH ({} gas * avg {} gwei)",
            paid.trim_end_matches('0'),
            total_gas,
            avg_gas_price.trim_end_matches('0').trim_end_matches('.')
        );

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
            script_config.collect_rpcs(&txs);

            if script_config.has_rpcs() {
                script_config.check_multi_chain_constraints(&libraries)?;

                let gas_filled_txs = if self.skip_simulation {
                    println!("\nSKIPPING ON CHAIN SIMULATION.");
                    txs.into_iter()
                        .map(|btx| {
                            let mut tx =
                                TransactionWithMetadata::from_typed_transaction(btx.transaction);
                            tx.rpc = btx.rpc;
                            tx
                        })
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
                    .bundle_transactions(gas_filled_txs, target, &mut script_config.config, returns)
                    .await?;

                if script_config.has_multiple_rpcs() {
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
                            result.script_wallets,
                            verify,
                        )
                        .await?;
                    }
                } else if self.broadcast {
                    let deployment_sequence = deployments.first_mut().expect("to be set.");
                    let rpc = script_config.total_rpcs.into_iter().next().expect("exists; qed");

                    deployment_sequence.add_libraries(libraries);

                    self.send_transactions(deployment_sequence, &rpc, &result.script_wallets)
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

    /// Returns all transactions in a list of [`ScriptSequence`]. List length will be higher than 1,
    /// if we're dealing with a multi chain deployment.
    ///
    /// Each transaction will be added with the correct transaction type and gas estimation.
    async fn bundle_transactions(
        &self,
        transactions: VecDeque<TransactionWithMetadata>,
        target: &ArtifactId,
        config: &mut Config,
        returns: HashMap<String, NestedValue>,
    ) -> eyre::Result<Vec<ScriptSequence>> {
        // User might be using both "in-code" forks and `--fork-url`.
        let arg_url = self.evm_opts.fork_url.clone().unwrap_or_default();
        let last_rpc = &transactions.back().expect("exists; qed").rpc;
        let is_multi_deployment = transactions.iter().any(|tx| &tx.rpc != last_rpc);

        let mut total_gas_per_rpc: HashMap<String, U256> = HashMap::new();

        // Batches sequence of transactions from different rpcs.
        let mut new_sequence = VecDeque::new();
        let mut manager = ProvidersManager::default();
        let mut deployments = vec![];

        // Peeks next transaction to figure out if it's the same rpc as the current batch.
        let mut txes_iter = transactions.into_iter().peekable();

        // Config is used to initialize the sequence chain, so we need to change when handling a new
        // sequence. This makes sure we don't lose the original value.
        let original_config_chain = config.chain_id;

        while let Some(mut tx) = txes_iter.next() {
            // Fills the RPC on the transaction in case it's missing one.
            let tx_rpc = tx.rpc.unwrap_or_else(|| arg_url.clone());
            if tx_rpc.is_empty() {
                eyre::bail!("Transaction needs an associated RPC if it is to be broadcasted.");
            }
            tx.rpc = Some(tx_rpc.clone());

            // Get or initialize the RPC provider.
            let provider_info = match manager.inner.entry(tx_rpc.clone()) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let info = ProviderInfo::new(&tx_rpc, &tx, self.legacy).await?;
                    entry.insert(info)
                }
            };

            // Handles chain specific requirements.
            tx.change_type(provider_info.is_legacy);
            tx.transaction.set_chain_id(provider_info.chain);

            if !self.skip_simulation {
                let typed_tx = tx.typed_tx_mut();

                if has_different_gas_calc(provider_info.chain) {
                    self.estimate_gas(typed_tx, &provider_info.provider).await?;
                }

                let total_gas = total_gas_per_rpc.entry(tx_rpc.clone()).or_insert(U256::zero());
                *total_gas += *typed_tx.gas().expect("gas is set");
            }

            // We only create the [`ScriptSequence`] object when we collect all the rpc related
            // transactions.
            if let Some(next_tx) = txes_iter.peek() {
                if next_tx.rpc == tx.rpc {
                    new_sequence.push_back(tx);
                    continue
                }
            }

            new_sequence.push_back(tx);

            config.chain_id = Some(provider_info.chain.into());
            let sequence = ScriptSequence::new(
                new_sequence,
                returns.clone(),
                &self.sig,
                target,
                config,
                self.broadcast,
                is_multi_deployment,
            )?;

            deployments.push(sequence);

            new_sequence = VecDeque::new();
        }

        // Restore previous config chain.
        config.chain_id = original_config_chain;

        if !self.skip_simulation {
            // Present gas information on a per RPC basis.
            for (rpc, total_gas) in total_gas_per_rpc {
                let provider_info = manager.inner.get(&rpc).expect("to be set.");

                // We don't store it in the transactions, since we want the most updated value.
                // Right before broadcasting.
                let per_gas = if let Some(gas_price) = self.with_gas_price {
                    gas_price
                } else if provider_info.is_legacy {
                    provider_info.gas_price.expect("to be set.")
                } else {
                    provider_info.eip1559_fees.expect("to be set.").0
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
