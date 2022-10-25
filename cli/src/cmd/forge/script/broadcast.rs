use super::{sequence::ScriptSequence, *};
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
use foundry_common::{estimate_eip1559_fees, try_get_http_provider, RetryProvider};
use foundry_config::Chain;
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::{cmp::min, ops::Mul, sync::Arc};

impl ScriptArgs {
    /// Sends the transactions which haven't been broadcasted yet.
    pub async fn send_transactions(
        &self,
        deployment_sequence: &mut ScriptSequence,
        fork_url: &str,
        script_wallets: Vec<LocalWallet>,
    ) -> eyre::Result<()> {
        let provider = Arc::new(try_get_http_provider(fork_url)?);
        let already_broadcasted = deployment_sequence.receipts.len();

        if already_broadcasted < deployment_sequence.transactions.len() {
            let required_addresses = deployment_sequence
                .typed_transactions()
                .into_iter()
                .skip(already_broadcasted)
                .map(|tx| *tx.from().expect("No sender for onchain transaction!"))
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
                        let fees = estimate_eip1559_fees(&provider, Some(chain))
                            .await
                            .wrap_err("Failed to estimate EIP1559 fees")?;

                        (None, Some(fees))
                    }
                }
            };

            // Iterate through transactions, matching the `from` field with the associated
            // wallet. Then send the transaction. Panics if we find a unknown `from`
            let sequence = deployment_sequence
                .typed_transactions()
                .into_iter()
                .skip(already_broadcasted)
                .map(|tx| {
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
        println!(
            "\nONCHAIN EXECUTION COMPLETE & SUCCESSFUL. Transaction receipts written to {:?}",
            deployment_sequence.path
        );

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
    ) -> eyre::Result<TxHash> {
        let from = tx.from().expect("no sender");

        if sequential_broadcast {
            let nonce = foundry_utils::next_nonce(*from, fork_url, None)
                .await
                .map_err(|_| eyre::eyre!("Not able to query the EOA nonce."))?;

            let tx_nonce = tx.nonce().expect("no nonce");

            if nonce != *tx_nonce {
                eyre::bail!("EOA nonce changed unexpectedly while sending transactions.")
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
        mut verify: VerifyBundle,
    ) -> eyre::Result<()> {
        if let Some(txs) = result.transactions {
            if let Some(fork_url) = script_config.evm_opts.fork_url.clone() {
                let gas_filled_txs = if self.skip_simulation {
                    println!("\nSKIPPING ON CHAIN SIMULATION.");
                    txs.into_iter().map(TransactionWithMetadata::from_typed_transaction).collect()
                } else {
                    self.execute_transactions(
                        txs,
                        &mut script_config,
                        decoder,
                        &verify.known_contracts,
                    )
                    .await
                    .wrap_err_with(|| {
                            "Transaction failed when running the on-chain simulation. Check the trace above for more information."
                    })?
                };

                let provider = Arc::new(try_get_http_provider(&fork_url)?);
                let chain = provider.get_chainid().await?.as_u64();

                verify.set_chain(&script_config.config, chain.into());

                let returns = self.get_returns(&script_config, &result.returned)?;

                let mut deployment_sequence = ScriptSequence::new(
                    self.handle_chain_requirements(gas_filled_txs, provider, chain).await?,
                    returns,
                    &self.sig,
                    target,
                    &script_config.config,
                    chain,
                    self.broadcast || self.resume,
                )?;

                deployment_sequence.add_libraries(libraries);

                if self.broadcast {
                    self.send_transactions(
                        &mut deployment_sequence,
                        &fork_url,
                        result.script_wallets,
                    )
                    .await?;
                    if self.verify {
                        deployment_sequence.verify_contracts(verify, chain).await?;
                    }
                } else {
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

    /// Modify each transaction according to the specific chain requirements (transaction type
    /// and/or gas calculations).
    async fn handle_chain_requirements(
        &self,
        txes: VecDeque<TransactionWithMetadata>,
        provider: Arc<RetryProvider>,
        chain: u64,
    ) -> eyre::Result<VecDeque<TransactionWithMetadata>> {
        let mut is_legacy = self.legacy;
        if let Chain::Named(chain) = Chain::from(chain) {
            is_legacy |= chain.is_legacy();
        };

        let mut new_txes = VecDeque::new();
        let mut total_gas = U256::zero();
        for mut tx in txes.into_iter() {
            tx.change_type(is_legacy);

            if !self.skip_simulation {
                let typed_tx = tx.typed_tx_mut();

                if has_different_gas_calc(chain) {
                    self.estimate_gas(typed_tx, &provider).await?;
                }

                total_gas += *typed_tx.gas().expect("gas is set");
            }

            new_txes.push_back(tx);
        }

        if !self.skip_simulation {
            // We don't store it in the transactions, since we want the most updated value. Right
            // before broadcasting.
            let per_gas = if let Some(gas_price) = self.with_gas_price {
                gas_price
            } else {
                match new_txes.front().unwrap().typed_tx() {
                    TypedTransaction::Legacy(_) | TypedTransaction::Eip2930(_) => {
                        provider.get_gas_price().await?
                    }
                    TypedTransaction::Eip1559(_) => provider.estimate_eip1559_fees(None).await?.0,
                }
            };

            println!("\n==========================");
            println!("\nEstimated total gas used for script: {}", total_gas);
            println!(
                "\nEstimated amount required: {} ETH",
                format_units(total_gas.saturating_mul(per_gas), 18)
                    .unwrap_or_else(|_| "[Could not calculate]".to_string())
                    .trim_end_matches('0')
            );
            println!("\n==========================");
        }
        Ok(new_txes)
    }
    /// Uses the signer to submit a transaction to the network. If it fails, it tries to retrieve
    /// the transaction hash that can be used on a later run with `--resume`.
    async fn broadcast<T, S>(
        &self,
        signer: &SignerMiddleware<T, S>,
        mut legacy_or_1559: TypedTransaction,
    ) -> eyre::Result<TxHash>
    where
        T: Middleware + 'static,
        S: Signer + 'static,
    {
        tracing::debug!("sending transaction: {:?}", legacy_or_1559);

        // Chains which use `eth_estimateGas` are being sent sequentially and require their gas to
        // be re-estimated right before broadcasting.
        if has_different_gas_calc(signer.signer().chain_id()) || self.skip_simulation {
            // if already set, some RPC endpoints might simply return the gas value that is already
            // set in the request and omit the estimate altogether, so we remove it here
            let _ = legacy_or_1559.gas_mut().take();

            self.estimate_gas(&mut legacy_or_1559, signer.provider()).await?;
        }

        // Signing manually so we skip `fill_transaction` and its `eth_createAccessList` request.
        let signature = signer
            .sign_transaction(
                &legacy_or_1559,
                *legacy_or_1559.from().expect("Tx should have a `from`."),
            )
            .await
            .wrap_err_with(|| "Failed to sign transaction")?;

        // Submit the raw transaction
        let pending =
            signer.provider().send_raw_transaction(legacy_or_1559.rlp_signed(&signature)).await?;

        Ok(pending.tx_hash())
    }

    async fn estimate_gas<T>(
        &self,
        tx: &mut TypedTransaction,
        provider: &Provider<T>,
    ) -> eyre::Result<()>
    where
        T: JsonRpcClient,
    {
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
