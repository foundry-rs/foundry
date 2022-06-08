use crate::{
    cmd::{
        forge::script::receipts::wait_for_receipts, has_different_gas_calc, ScriptSequence,
        VerifyBundle,
    },
    init_progress,
    opts::WalletType,
    update_progress,
    utils::get_http_provider,
};
use ethers::{
    prelude::{Http, Provider, RetryClient, Signer, SignerMiddleware, TxHash},
    providers::Middleware,
    types::{transaction::eip2718::TypedTransaction, Chain},
    utils::format_units,
};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use std::{cmp::min, fmt, sync::Arc};

use super::*;

impl ScriptArgs {
    /// Sends the transactions which haven't been broadcasted yet.
    pub async fn send_transactions(
        &self,
        deployment_sequence: &mut ScriptSequence,
        fork_url: &str,
    ) -> eyre::Result<()> {
        let provider = get_http_provider(fork_url, true);
        let already_broadcasted = deployment_sequence.receipts.len();

        if already_broadcasted < deployment_sequence.transactions.len() {
            let required_addresses = deployment_sequence
                .transactions
                .iter()
                .skip(already_broadcasted)
                .map(|tx| *tx.from().expect("No sender for onchain transaction!"))
                .collect();

            let local_wallets = self.wallets.find_all(provider.clone(), required_addresses).await?;
            if local_wallets.is_empty() {
                eyre::bail!("Error accessing local wallet when trying to send onchain transaction, did you set a private key, mnemonic or keystore?")
            }

            // We only wait for a transaction receipt before sending the next transaction, if there
            // is more than one signer. There would be no way of assuring their order
            // otherwise.
            let sequential_broadcast = local_wallets.len() != 1 || self.slow;

            // Make a one-time gas price estimation
            let (gas_price, eip1559_fees) = {
                match deployment_sequence.transactions.front().unwrap() {
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
                .transactions
                .iter()
                .skip(already_broadcasted)
                .map(|tx| {
                    let from = *tx.from().expect("No sender for onchain transaction!");
                    let signer = local_wallets.get(&from).expect("`find_all` returned incomplete.");

                    let mut tx = tx.clone();

                    tx.set_chain_id(signer.chain_id());

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
                        update_progress!(pb, (index + already_broadcasted));
                        index += 1;

                        wait_for_receipts(
                            vec![tx_hash.await?],
                            deployment_sequence,
                            provider.clone(),
                        )
                        .await?;
                    } else {
                        pending_transactions.push(tx_hash);
                    }
                }

                if !pending_transactions.is_empty() {
                    let mut buffer = futures::stream::iter(pending_transactions).buffered(7);

                    let mut tx_hashes = vec![];

                    while let Some(tx_hash) = buffer.next().await {
                        update_progress!(pb, (index + already_broadcasted));
                        index += 1;
                        let tx_hash = tx_hash?;
                        deployment_sequence.add_pending(tx_hash);
                        tx_hashes.push(tx_hash);
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
            WalletType::Local(signer) => broadcast(signer, tx).await,
            WalletType::Ledger(signer) => broadcast(signer, tx).await,
            WalletType::Trezor(signer) => broadcast(signer, tx).await,
        }
    }

    /// Executes the passed transactions in sequence, and if no error has occurred, broadcasts
    /// them.
    pub async fn handle_broadcastable_transactions(
        &self,
        target: &ArtifactId,
        transactions: Option<VecDeque<TypedTransaction>>,
        libraries: Libraries,
        decoder: &mut CallTraceDecoder,
        script_config: &ScriptConfig,
        verify: VerifyBundle,
    ) -> eyre::Result<()> {
        if let Some(txs) = transactions {
            if script_config.evm_opts.fork_url.is_some() {
                let (gas_filled_txs, create2_contracts) =
                    self.execute_transactions(txs, script_config, decoder).await.map_err(|_| {
                        eyre::eyre!(
                            "One or more transactions failed when simulating the
                on-chain version. Check the trace by re-running with `-vvv`"
                        )
                    })?;

                let fork_url = self.evm_opts.fork_url.as_ref().unwrap().clone();

                let provider = get_http_provider(&fork_url, false);
                let chain = provider.get_chainid().await?.as_u64();

                let mut deployment_sequence = ScriptSequence::new(
                    self.handle_chain_requirements(gas_filled_txs, provider, chain).await?,
                    &self.sig,
                    target,
                    &script_config.config,
                    chain,
                )?;

                deployment_sequence.add_libraries(libraries);

                create2_contracts
                    .into_iter()
                    .for_each(|addr| deployment_sequence.add_create2(addr));

                if self.broadcast {
                    self.send_transactions(&mut deployment_sequence, &fork_url).await?;
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
        txes: VecDeque<TypedTransaction>,
        provider: Arc<Provider<RetryClient<Http>>>,
        chain: u64,
    ) -> eyre::Result<VecDeque<TypedTransaction>> {
        let is_legacy =
            self.legacy || Chain::try_from(chain).map(|x| Chain::is_legacy(&x)).unwrap_or_default();

        let mut new_txes: VecDeque<TypedTransaction> = VecDeque::new();
        let mut total_gas = U256::zero();
        for tx in txes.into_iter() {
            let mut tx = if is_legacy {
                TypedTransaction::Legacy(tx.into())
            } else {
                TypedTransaction::Eip1559(tx.into())
            };

            if has_different_gas_calc(chain) {
                tx.set_gas(provider.estimate_gas(&tx).await?);
            }

            total_gas += *tx.gas().expect("");

            new_txes.push_back(tx);
        }

        // We don't store it in the transactions, since we want the most updated value. Right before
        // broadcasting.
        let per_gas = {
            match new_txes.front().unwrap() {
                TypedTransaction::Legacy(_) | TypedTransaction::Eip2930(_) => {
                    provider.get_gas_price().await?
                }
                TypedTransaction::Eip1559(_) => provider.estimate_eip1559_fees(None).await?.0,
            }
        };
        println!("\n==========================");
        println!("\nEstimated total gas used for script: {}", total_gas);
        println!(
            "\nAmount required: {} ETH",
            format_units(total_gas.saturating_mul(per_gas), 18)
                .unwrap_or_else(|_| "[Could not calculate]".to_string())
                .trim_end_matches('0')
        );
        println!("\n==========================");
        Ok(new_txes)
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

/// Uses the signer to submit a transaction to the network. If it fails, it tries to retrieve the
/// transaction hash that can be used on a later run with `--resume`.
async fn broadcast<T, U>(
    signer: &SignerMiddleware<T, U>,
    legacy_or_1559: TypedTransaction,
) -> Result<TxHash, BroadcastError>
where
    T: Middleware,
    U: Signer,
{
    tracing::debug!("sending transaction: {:?}", legacy_or_1559);

    // Signing manually so we skip `fill_transaction` and its `eth_createAccessList` request.
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
