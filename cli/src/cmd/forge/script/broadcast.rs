use crate::{
    cmd::{forge::script::receipts::wait_for_receipts, ScriptSequence, VerifyBundle},
    opts::WalletType,
    utils::get_http_provider,
};
use ethers::{
    prelude::{SignerMiddleware, TxHash},
    providers::Middleware,
    types::{transaction::eip2718::TypedTransaction, Chain, TransactionReceipt},
};
use std::fmt;

use super::*;

impl ScriptArgs {
    pub async fn send_transactions(
        &self,
        deployment_sequence: &mut ScriptSequence,
        fork_url: &str,
    ) -> eyre::Result<()> {
        let provider = get_http_provider(fork_url);

        if deployment_sequence.receipts.len() < deployment_sequence.transactions.len() {
            let required_addresses = deployment_sequence
                .transactions
                .iter()
                .skip(deployment_sequence.receipts.len())
                .map(|tx| *tx.from().expect("No sender for onchain transaction!"))
                .collect();

            let local_wallets = self.wallets.find_all(&provider, required_addresses).await?;
            if local_wallets.is_empty() {
                eyre::bail!("Error accessing local wallet when trying to send onchain transaction, did you set a private key, mnemonic or keystore?")
            }

            // We only wait for a transaction receipt before sending the next transaction, if there
            // is more than one signer. There would be no way of assuring their order
            // otherwise.
            let sequential_broadcast = local_wallets.len() != 1 || self.slow;

            let transactions = deployment_sequence.transactions.clone();

            // Iterate through transactions, matching the `from` field with the associated
            // wallet. Then send the transaction. Panics if we find a unknown `from`
            let sequence = transactions.into_iter().map(|tx| {
                let from = *tx.from().expect("No sender for onchain transaction!");
                let signer = local_wallets.get(&from).expect("`find_all` returned incomplete.");
                (tx, signer)
            });

            let mut future_receipts = vec![];

            println!("##\nSending transactions.");
            for (tx, signer) in sequence {
                let receipt = self.send_transaction(tx, signer, sequential_broadcast, fork_url);

                if sequential_broadcast {
                    wait_for_receipts(vec![receipt], deployment_sequence).await?;
                } else {
                    future_receipts.push(receipt);
                }
            }

            if !sequential_broadcast {
                wait_for_receipts(future_receipts, deployment_sequence).await?;
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
    ) -> Result<(TransactionReceipt, U256), BroadcastError> {
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

    /// Executes the passed transactions in sequence, and if no error has occurred, it broadcasts
    /// them.
    pub async fn handle_broadcastable_transactions(
        &self,
        target: &ArtifactId,
        transactions: Option<VecDeque<TypedTransaction>>,
        decoder: &mut CallTraceDecoder,
        script_config: &ScriptConfig,
        verify: VerifyBundle,
    ) -> eyre::Result<()> {
        if let Some(txs) = transactions {
            if script_config.evm_opts.fork_url.is_some() {
                let (gas_filled_txs, create2_contracts) =
                    self.execute_transactions(txs, script_config, decoder)
                    .await
                    .map_err(|_| eyre::eyre!("One or more transactions failed when simulating the on-chain version. Check the trace by re-running with `-vvv`"))?;
                let fork_url = self.evm_opts.fork_url.as_ref().unwrap().clone();

                let provider = get_http_provider(&fork_url);
                let chain = provider.get_chainid().await?.as_u64();
                let is_legacy = self.legacy ||
                    Chain::try_from(chain).map(|x| Chain::is_legacy(&x)).unwrap_or_default();

                let txes = gas_filled_txs
                    .into_iter()
                    .map(|tx| {
                        let mut tx = if is_legacy {
                            TypedTransaction::Legacy(tx.into())
                        } else {
                            TypedTransaction::Eip1559(tx.into())
                        };
                        tx.set_chain_id(chain);
                        tx
                    })
                    .collect();

                let mut deployment_sequence =
                    ScriptSequence::new(txes, &self.sig, target, &script_config.config, chain)?;

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
) -> Result<(TransactionReceipt, U256), BroadcastError>
where
    SignerMiddleware<T, U>: Middleware,
{
    tracing::debug!("sending transaction: {:?}", legacy_or_1559);
    let nonce = *legacy_or_1559.nonce().unwrap();
    let pending = signer
        .send_transaction(legacy_or_1559, None)
        .await
        .map_err(|err| BroadcastError::Simple(err.to_string()))?;

    let tx_hash = pending.tx_hash();

    let receipt = match pending.await {
        Ok(receipt) => match receipt {
            Some(receipt) => receipt,
            None => {
                return Err(BroadcastError::ErrorWithTxHash(
                    format!("Didn't receive a receipt for {}", tx_hash),
                    tx_hash,
                ))
            }
        },
        Err(err) => return Err(BroadcastError::ErrorWithTxHash(err.to_string(), tx_hash)),
    };
    Ok((receipt, nonce))
}
