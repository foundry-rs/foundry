use crate::{
    cmd::ScriptSequence,
    utils::{get_http_provider, print_receipt},
};

use std::str::FromStr;

use ethers::{
    prelude::{k256::ecdsa::SigningKey, Http, Provider, SignerMiddleware, Wallet},
    providers::Middleware,
    signers::Signer,
    types::{transaction::eip2718::TypedTransaction, Address, Chain, TransactionReceipt},
};
use futures::future::join_all;

use super::*;

impl ScriptArgs {
    pub async fn send_transactions(
        &self,
        deployment_sequence: &mut ScriptSequence,
        fork_url: &str,
    ) -> eyre::Result<()> {
        let provider = get_http_provider(fork_url);
        let chain = provider.get_chainid().await?.as_u64();

        let local_wallets = self.wallets.all(chain)?;
        if local_wallets.is_empty() {
            eyre::bail!("Error accessing local wallet when trying to send onchain transaction, did you set a private key, mnemonic or keystore?")
        }

        let is_legacy =
            self.legacy || Chain::try_from(chain).map(|x| Chain::is_legacy(&x)).unwrap_or_default();

        // Iterate through transactions, matching the `from` field with the associated
        // wallet. Then send the transaction. Panics if we find a unknown `from`
        let sequence =
        deployment_sequence.transactions.iter().skip(deployment_sequence.receipts.len()).map(|tx| {
                let from = *tx.from().expect("No sender for onchain transaction!");
                if let Some(wallet) =
                    local_wallets.iter().find(|wallet| (**wallet).address() == from)
                {
                    let signer = SignerMiddleware::new(provider.clone(), wallet.clone());
                    Ok((tx, signer))
                } else {
                    let mut err_msg = format!(
                        "No associated wallet for address: {:?}. Unlocked wallets: {:?}",
                        from,
                        local_wallets
                            .iter()
                            .map(|wallet| wallet.address())
                            .collect::<Vec<Address>>()
                    );

                    // This is an actual used address
                    if from == Address::from_str(Config::DEFAULT_SENDER).unwrap() {
                        err_msg += "\nYou seem to be using Foundry's default sender. Be sure to set your own --sender."
                    }

                    eyre::bail!(err_msg)
                }
            });

        let mut future_receipts = vec![];
        let mut receipts = vec![];

        // We only wait for a transaction receipt before sending the next transaction, if there is
        // more than one signer. There would be no way of assuring their order otherwise.
        let wait = local_wallets.len() > 1;
        for payload in sequence {
            let (tx, signer) = payload?;
            let receipt =
                self.send_transaction(tx.clone(), signer, wait, chain, is_legacy, fork_url);
            if !wait {
                future_receipts.push(receipt);
            } else {
                let (receipt, nonce) = receipt.await?;
                print_receipt(&receipt, nonce)?;
                receipts.push(receipt);
            }
        }

        if !wait {
            deployment_sequence.set_receipts(receipts)
        } else {
            deployment_sequence.set_receipts(self.wait_for_receipts(future_receipts).await?)
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
        signer: SignerMiddleware<Provider<Http>, Wallet<SigningKey>>,
        wait: bool,
        chain: u64,
        is_legacy: bool,
        fork_url: &str,
    ) -> eyre::Result<(TransactionReceipt, U256)> {
        let mut legacy_or_1559 = if is_legacy {
            TypedTransaction::Legacy(tx.into())
        } else {
            TypedTransaction::Eip1559(tx.into())
        };
        legacy_or_1559.set_chain_id(chain);

        let from = *legacy_or_1559.from().expect("no sender");

        if wait {
            let nonce = foundry_utils::next_nonce(from, fork_url, None)
                .map_err(|_| eyre::eyre!("Not able to query the EOA nonce."))?;

            let tx_nonce = *legacy_or_1559.nonce().expect("no nonce");

            if nonce != tx_nonce {
                eyre::bail!("EOA nonce changed unexpectedly while sending transactions.");
            }
        }

        async fn broadcast<T, U>(
            signer: SignerMiddleware<T, U>,
            legacy_or_1559: TypedTransaction,
        ) -> eyre::Result<Option<TransactionReceipt>>
        where
            SignerMiddleware<T, U>: Middleware,
        {
            tracing::debug!("sending transaction: {:?}", legacy_or_1559);
            match signer.send_transaction(legacy_or_1559, None).await {
                Ok(pending) => pending.await.map_err(|e| eyre::eyre!(e)),
                Err(e) => Err(eyre::eyre!(e.to_string())),
            }
        }

        let nonce = *legacy_or_1559.nonce().expect("no nonce");
        let receipt = match broadcast(signer, legacy_or_1559).await {
            Ok(Some(res)) => (res, nonce),

            Ok(None) => {
                // todo what if it has been actually sent
                eyre::bail!("Failed to get transaction receipt?")
            }
            Err(e) => {
                eyre::bail!("Aborting! A transaction failed to send: {:#?}", e)
            }
        };

        Ok(receipt)
    }

    /// Executes the passed transactions in sequence, and if no error has occurred, it broadcasts
    /// them.
    pub async fn handle_broadcastable_transactions(
        &self,
        target: &ArtifactId,
        transactions: Option<VecDeque<TypedTransaction>>,
        decoder: &mut CallTraceDecoder,
        script_config: &ScriptConfig,
    ) -> eyre::Result<()> {
        if let Some(txs) = transactions {
            if script_config.evm_opts.fork_url.is_some() {
                let gas_filled_txs =
                    self.execute_transactions(txs, script_config, decoder)
                    .await
                    .map_err(|_| eyre::eyre!("One or more transactions failed when simulating the on-chain version. Check the trace by re-running with `-vvv`"))?;
                let fork_url = self.evm_opts.fork_url.as_ref().unwrap().clone();

                let provider = get_http_provider(&fork_url);
                let chain = provider.get_chainid().await?.as_u64();

                let mut deployment_sequence = ScriptSequence::new(
                    gas_filled_txs,
                    &self.sig,
                    target,
                    &script_config.config,
                    chain,
                )?;

                if self.broadcast {
                    self.send_transactions(&mut deployment_sequence, &fork_url).await?;
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

    async fn wait_for_receipts(
        &self,
        tasks: Vec<impl futures::Future<Output = eyre::Result<(TransactionReceipt, U256)>>>,
    ) -> eyre::Result<Vec<TransactionReceipt>> {
        let res = join_all(tasks).await;

        let mut err = None;
        let mut receipts = vec![];

        for receipt in res {
            match receipt {
                Ok(v) => receipts.push(v),
                Err(e) => {
                    err = Some(e);
                    break
                }
            };
        }

        // Receipts may have arrived out of order
        receipts.sort_by(|a, b| a.1.cmp(&b.1));
        for (receipt, nonce) in &receipts {
            print_receipt(receipt, *nonce)?;
        }

        if let Some(err) = err {
            Err(err)
        } else {
            Ok(receipts.into_iter().map(|(receipt, _)| receipt).collect())
        }
    }
}
