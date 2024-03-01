use super::{
    build::LinkedBuildData,
    execute::{ExecutionArtifacts, ExecutionData},
    receipts::{self, clear_pendings},
    sequence::{ScriptSequence, ScriptSequenceKind},
    simulate::BundledState,
    verify::VerifyBundle,
    ScriptArgs, ScriptConfig,
};
use alloy_primitives::{utils::format_units, Address, TxHash, U256};
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_providers::{JsonRpcClient, Middleware, Provider};
use ethers_signers::Signer;
use eyre::{bail, Context, Result};
use foundry_cli::{
    init_progress, update_progress,
    utils::{has_batch_support, has_different_gas_calc},
};
use foundry_common::{
    provider::ethers::{
        estimate_eip1559_fees, get_http_provider, try_get_http_provider, RetryProvider,
    },
    shell,
    types::{ToAlloy, ToEthers},
};
use foundry_wallets::WalletSigner;
use futures::{future::join_all, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

async fn estimate_gas<T>(
    tx: &mut TypedTransaction,
    provider: &Provider<T>,
    estimate_multiplier: u64,
) -> Result<()>
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
            estimate_multiplier /
            100,
    );
    Ok(())
}

pub async fn send_transaction(
    provider: Arc<RetryProvider>,
    mut tx: TypedTransaction,
    kind: SendTransactionKind<'_>,
    sequential_broadcast: bool,
    is_fixed_gas_limit: bool,
    skip_simulation: bool,
    estimate_multiplier: u64,
) -> Result<TxHash> {
    let from = tx.from().expect("no sender");

    if sequential_broadcast {
        let nonce = provider.get_transaction_count(*from, None).await?;

        let tx_nonce = tx.nonce().expect("no nonce");
        if nonce != *tx_nonce {
            bail!("EOA nonce changed unexpectedly while sending transactions. Expected {tx_nonce} got {nonce} from provider.")
        }
    }

    // Chains which use `eth_estimateGas` are being sent sequentially and require their
    // gas to be re-estimated right before broadcasting.
    if !is_fixed_gas_limit &&
        (has_different_gas_calc(provider.get_chainid().await?.as_u64()) || skip_simulation)
    {
        estimate_gas(&mut tx, &provider, estimate_multiplier).await?;
    }

    let pending = match kind {
        SendTransactionKind::Unlocked(addr) => {
            debug!("sending transaction from unlocked account {:?}: {:?}", addr, tx);

            // Submit the transaction
            provider.send_transaction(tx, None).await?
        }
        SendTransactionKind::Raw(signer) => {
            debug!("sending transaction: {:?}", tx);

            // Signing manually so we skip `fill_transaction` and its `eth_createAccessList`
            // request.
            let signature =
                signer.sign_transaction(&tx).await.wrap_err("Failed to sign transaction")?;

            // Submit the raw transaction
            provider.send_raw_transaction(tx.rlp_signed(&signature)).await?
        }
    };

    Ok(pending.tx_hash().to_alloy())
}

/// How to send a single transaction
#[derive(Clone)]
pub enum SendTransactionKind<'a> {
    Unlocked(Address),
    Raw(&'a WalletSigner),
}

/// Represents how to send _all_ transactions
pub enum SendTransactionsKind {
    /// Send via `eth_sendTransaction` and rely on the  `from` address being unlocked.
    Unlocked(HashSet<Address>),
    /// Send a signed transaction via `eth_sendRawTransaction`
    Raw(HashMap<Address, WalletSigner>),
}

impl SendTransactionsKind {
    /// Returns the [`SendTransactionKind`] for the given address
    ///
    /// Returns an error if no matching signer is found or the address is not unlocked
    pub fn for_sender(&self, addr: &Address) -> Result<SendTransactionKind<'_>> {
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
    pub fn signers_count(&self) -> usize {
        match self {
            SendTransactionsKind::Unlocked(addr) => addr.len(),
            SendTransactionsKind::Raw(signers) => signers.len(),
        }
    }
}

impl BundledState {
    pub async fn wait_for_pending(mut self) -> Result<Self> {
        let futs = self
            .sequence
            .iter_sequeneces_mut()
            .map(|sequence| async move {
                let rpc_url = sequence.rpc_url();
                let provider = Arc::new(get_http_provider(rpc_url));
                receipts::wait_for_pending(provider, sequence).await
            })
            .collect::<Vec<_>>();

        let errors =
            join_all(futs).await.into_iter().filter(|res| res.is_err()).collect::<Vec<_>>();

        if !errors.is_empty() {
            return Err(eyre::eyre!("{errors:?}"));
        }

        Ok(self)
    }

    pub async fn broadcast(mut self) -> Result<BroadcastedState> {
        let required_addresses = self
            .sequence
            .iter_sequences()
            .flat_map(|sequence| {
                sequence
                    .typed_transactions()
                    .map(|tx| (*tx.from().expect("No sender for onchain transaction!")).to_alloy())
            })
            .collect::<HashSet<_>>();

        let send_kind = if self.args.unlocked {
            SendTransactionsKind::Unlocked(required_addresses)
        } else {
            let signers = self.script_wallets.into_multi_wallet().into_signers()?;
            let mut missing_addresses = Vec::new();

            for addr in &required_addresses {
                if !signers.contains_key(addr) {
                    missing_addresses.push(addr);
                }
            }

            if !missing_addresses.is_empty() {
                eyre::bail!(
                    "No associated wallet for addresses: {:?}. Unlocked wallets: {:?}",
                    missing_addresses,
                    signers.keys().collect::<Vec<_>>()
                );
            }

            SendTransactionsKind::Raw(signers)
        };

        for sequence in self.sequence.iter_sequeneces_mut() {
            let provider = Arc::new(try_get_http_provider(sequence.rpc_url())?);
            let already_broadcasted = sequence.receipts.len();

            if already_broadcasted < sequence.transactions.len() {
                let chain = provider.get_chainid().await?.as_u64();

                // We only wait for a transaction receipt before sending the next transaction, if
                // there is more than one signer. There would be no way of assuring
                // their order otherwise. Or if the chain does not support batched
                // transactions (eg. Arbitrum).
                let sequential_broadcast =
                    send_kind.signers_count() != 1 || self.args.slow || !has_batch_support(chain);

                // Make a one-time gas price estimation
                let (gas_price, eip1559_fees) = match self.args.with_gas_price {
                    None => match sequence.transactions.front().unwrap().typed_tx() {
                        TypedTransaction::Eip1559(_) => {
                            let mut fees = estimate_eip1559_fees(&provider, Some(chain))
                                .await
                                .wrap_err("Failed to estimate EIP1559 fees. This chain might not support EIP1559, try adding --legacy to your command.")?;

                            if let Some(priority_gas_price) = self.args.priority_gas_price {
                                fees.1 = priority_gas_price.to_ethers();
                            }

                            (None, Some(fees))
                        }
                        _ => (provider.get_gas_price().await.ok(), None),
                    },
                    Some(gas_price) => (Some(gas_price.to_ethers()), None),
                };

                // Iterate through transactions, matching the `from` field with the associated
                // wallet. Then send the transaction. Panics if we find a unknown `from`
                let transactions = sequence
                    .transactions
                    .iter()
                    .skip(already_broadcasted)
                    .map(|tx_with_metadata| {
                        let tx = tx_with_metadata.typed_tx();
                        let from =
                            (*tx.from().expect("No sender for onchain transaction!")).to_alloy();

                        let kind = send_kind.for_sender(&from)?;
                        let is_fixed_gas_limit = tx_with_metadata.is_fixed_gas_limit;

                        let mut tx = tx.clone();

                        tx.set_chain_id(chain);

                        if let Some(gas_price) = gas_price {
                            tx.set_gas_price(gas_price);
                        } else {
                            let eip1559_fees = eip1559_fees.expect("was set above");
                            // fill gas price
                            match tx {
                                TypedTransaction::Eip1559(ref mut inner) => {
                                    inner.max_priority_fee_per_gas = Some(eip1559_fees.1);
                                    inner.max_fee_per_gas = Some(eip1559_fees.0);
                                }
                                _ => {
                                    // If we're here, it means that first transaction of the
                                    // sequence was EIP1559 transaction (see match statement above),
                                    // however, we can only have transactions of the same type in
                                    // the sequence.
                                    unreachable!()
                                }
                            }
                        }

                        Ok((tx, kind, is_fixed_gas_limit))
                    })
                    .collect::<Result<Vec<_>>>()?;

                let pb = init_progress!(transactions, "txes");

                // We send transactions and wait for receipts in batches of 100, since some networks
                // cannot handle more than that.
                let batch_size = if sequential_broadcast { 1 } else { 100 };
                let mut index = 0;

                for (batch_number, batch) in
                    transactions.chunks(batch_size).map(|f| f.to_vec()).enumerate()
                {
                    let mut pending_transactions = vec![];

                    shell::println(format!(
                        "##\nSending transactions [{} - {}].",
                        batch_number * batch_size,
                        batch_number * batch_size + std::cmp::min(batch_size, batch.len()) - 1
                    ))?;
                    for (tx, kind, is_fixed_gas_limit) in batch.into_iter() {
                        let tx_hash = send_transaction(
                            provider.clone(),
                            tx,
                            kind,
                            sequential_broadcast,
                            is_fixed_gas_limit,
                            self.args.skip_simulation,
                            self.args.gas_estimate_multiplier,
                        );
                        pending_transactions.push(tx_hash);
                    }

                    if !pending_transactions.is_empty() {
                        let mut buffer = futures::stream::iter(pending_transactions).buffered(7);

                        while let Some(tx_hash) = buffer.next().await {
                            let tx_hash = tx_hash?;
                            sequence.add_pending(index, tx_hash);

                            update_progress!(pb, (index + already_broadcasted));
                            index += 1;
                        }

                        // Checkpoint save
                        sequence.save(true)?;

                        shell::println("##\nWaiting for receipts.")?;
                        receipts::clear_pendings(provider.clone(), sequence, None).await?;
                    }

                    // Checkpoint save
                    sequence.save(true)?;
                }
            }

            shell::println("\n\n==========================")?;
            shell::println("\nONCHAIN EXECUTION COMPLETE & SUCCESSFUL.")?;

            let (total_gas, total_gas_price, total_paid) = sequence.receipts.iter().fold(
                (U256::ZERO, U256::ZERO, U256::ZERO),
                |acc, receipt| {
                    let gas_used = receipt.gas_used.unwrap_or_default().to_alloy();
                    let gas_price = receipt.effective_gas_price.unwrap_or_default().to_alloy();
                    (acc.0 + gas_used, acc.1 + gas_price, acc.2 + gas_used * gas_price)
                },
            );
            let paid = format_units(total_paid, 18).unwrap_or_else(|_| "N/A".to_string());
            let avg_gas_price =
                format_units(total_gas_price / U256::from(sequence.receipts.len()), 9)
                    .unwrap_or_else(|_| "N/A".to_string());

            shell::println(format!(
                "Total Paid: {} ETH ({} gas * avg {} gwei)",
                paid.trim_end_matches('0'),
                total_gas,
                avg_gas_price.trim_end_matches('0').trim_end_matches('.')
            ))?;
        }

        Ok(BroadcastedState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            execution_data: self.execution_data,
            execution_artifacts: self.execution_artifacts,
            sequence: self.sequence,
        })
    }
}

pub struct BroadcastedState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub build_data: LinkedBuildData,
    pub execution_data: ExecutionData,
    pub execution_artifacts: ExecutionArtifacts,
    pub sequence: ScriptSequenceKind,
}
