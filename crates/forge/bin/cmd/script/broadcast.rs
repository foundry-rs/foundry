use super::{receipts::clear_pendings, sequence::{ScriptSequence, ScriptSequenceKind},
    simulate::BundledState, verify::VerifyBundle, ScriptArgs, ScriptConfig,
};
use alloy_primitives::{utils::format_units, Address, TxHash, U256};
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_providers::{JsonRpcClient, Middleware, Provider};
use ethers_signers::Signer;
use eyre::{bail, Context, ContextCompat, Result};
use foundry_cli::{
    init_progress, update_progress,
    utils::{has_batch_support, has_different_gas_calc},
};
use foundry_common::{
    provider::ethers::{estimate_eip1559_fees, try_get_http_provider, RetryProvider},
    shell,
    types::{ToAlloy, ToEthers},
};
use foundry_compilers::artifacts::Libraries;
use foundry_config::Config;
use foundry_wallets::WalletSigner;
use futures::StreamExt;
use std::{
    cmp::min,
    collections::{HashMap, HashSet},
    sync::Arc,
};

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
        if self.broadcast {
            match &mut state.sequence {
                ScriptSequenceKind::Multi(sequence) => {
                    trace!(target: "script", "broadcasting multi chain deployment");
                    self.multi_chain_deployment(
                        sequence,
                        state.build_data.libraries,
                        &state.script_config.config,
                        verify,
                        &state.script_config.script_wallets.into_multi_wallet().into_signers()?,
                    )
                    .await?;
                }
                ScriptSequenceKind::Single(sequence) => {
                    self.single_deployment(
                        sequence,
                        state.script_config,
                        state.build_data.libraries,
                        verify,
                    )
                    .await?;
                }
            }
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
