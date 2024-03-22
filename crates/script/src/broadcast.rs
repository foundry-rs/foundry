use crate::{
    build::LinkedBuildData, sequence::ScriptSequenceKind, verify::BroadcastedState, ScriptArgs,
    ScriptConfig,
};

use super::receipts;
use alloy_primitives::{utils::format_units, Address, TxHash, U256};
use ethers_core::types::{transaction::eip2718::TypedTransaction, BlockId};
use ethers_providers::{JsonRpcClient, Middleware, Provider};
use ethers_signers::Signer;
use eyre::{bail, Context, Result};
use forge_verify::provider::VerificationProviderType;
use foundry_cheatcodes::ScriptWallets;
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
use foundry_config::Config;
use foundry_wallets::WalletSigner;
use futures::{future::join_all, StreamExt};
use itertools::Itertools;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub async fn estimate_gas<T>(
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

pub async fn next_nonce(
    caller: Address,
    provider_url: &str,
    block: Option<BlockId>,
) -> eyre::Result<u64> {
    let provider = Provider::try_from(provider_url)
        .wrap_err_with(|| format!("bad fork_url provider: {provider_url}"))?;
    let res = provider.get_transaction_count(caller.to_ethers(), block).await?.to_alloy();
    res.try_into().map_err(Into::into)
}

pub async fn send_transaction(
    provider: Arc<RetryProvider>,
    mut tx: TypedTransaction,
    kind: SendTransactionKind<'_>,
    sequential_broadcast: bool,
    is_fixed_gas_limit: bool,
    estimate_via_rpc: bool,
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
    if !is_fixed_gas_limit && estimate_via_rpc {
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

/// State after we have bundled all [TransactionWithMetadata] objects into a single
/// [ScriptSequenceKind] object containing one or more script sequences.
pub struct BundledState {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig,
    pub script_wallets: ScriptWallets,
    pub build_data: LinkedBuildData,
    pub sequence: ScriptSequenceKind,
}

impl BundledState {
    pub async fn wait_for_pending(mut self) -> Result<Self> {
        let futs = self
            .sequence
            .sequences_mut()
            .iter_mut()
            .map(|sequence| async move {
                let rpc_url = sequence.rpc_url();
                let provider = Arc::new(get_http_provider(rpc_url));
                receipts::wait_for_pending(provider, sequence).await
            })
            .collect::<Vec<_>>();

        let errors = join_all(futs).await.into_iter().filter_map(Result::err).collect::<Vec<_>>();

        self.sequence.save(true, false)?;

        if !errors.is_empty() {
            return Err(eyre::eyre!("{}", errors.iter().format("\n")));
        }

        Ok(self)
    }

    /// Broadcasts transactions from all sequences.
    pub async fn broadcast(mut self) -> Result<BroadcastedState> {
        let required_addresses = self
            .sequence
            .sequences()
            .iter()
            .flat_map(|sequence| {
                sequence
                    .typed_transactions()
                    .map(|tx| (*tx.from().expect("No sender for onchain transaction!")).to_alloy())
            })
            .collect::<HashSet<_>>();

        if required_addresses.contains(&Config::DEFAULT_SENDER) {
            eyre::bail!(
                "You seem to be using Foundry's default sender. Be sure to set your own --sender."
            );
        }

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

        for i in 0..self.sequence.sequences().len() {
            let mut sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

            let provider = Arc::new(try_get_http_provider(sequence.rpc_url())?);
            let already_broadcasted = sequence.receipts.len();

            if already_broadcasted < sequence.transactions.len() {
                // Make a one-time gas price estimation
                let (gas_price, eip1559_fees) = match self.args.with_gas_price {
                    None => match sequence.transactions.front().unwrap().typed_tx() {
                        TypedTransaction::Eip1559(_) => {
                            let mut fees = estimate_eip1559_fees(&provider, Some(sequence.chain))
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

                        tx.set_chain_id(sequence.chain);

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

                let estimate_via_rpc =
                    has_different_gas_calc(sequence.chain) || self.args.skip_simulation;

                // We only wait for a transaction receipt before sending the next transaction, if
                // there is more than one signer. There would be no way of assuring
                // their order otherwise.
                // Or if the chain does not support batched transactions (eg. Arbitrum).
                // Or if we need to invoke eth_estimateGas before sending transactions.
                let sequential_broadcast = estimate_via_rpc ||
                    self.args.slow ||
                    send_kind.signers_count() != 1 ||
                    !has_batch_support(sequence.chain);

                let pb = init_progress!(transactions, "txes");

                // We send transactions and wait for receipts in batches of 100, since some networks
                // cannot handle more than that.
                let batch_size = if sequential_broadcast { 1 } else { 100 };
                let mut index = already_broadcasted;

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
                            estimate_via_rpc,
                            self.args.gas_estimate_multiplier,
                        );
                        pending_transactions.push(tx_hash);
                    }

                    if !pending_transactions.is_empty() {
                        let mut buffer = futures::stream::iter(pending_transactions).buffered(7);

                        while let Some(tx_hash) = buffer.next().await {
                            let tx_hash = tx_hash.wrap_err("Failed to send transaction")?;
                            sequence.add_pending(index, tx_hash);

                            // Checkpoint save
                            self.sequence.save(true, false)?;
                            sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

                            update_progress!(pb, index - already_broadcasted);
                            index += 1;
                        }

                        // Checkpoint save
                        self.sequence.save(true, false)?;
                        sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

                        shell::println("##\nWaiting for receipts.")?;
                        receipts::clear_pendings(provider.clone(), sequence, None).await?;
                    }
                    // Checkpoint save
                    self.sequence.save(true, false)?;
                    sequence = self.sequence.sequences_mut().get_mut(i).unwrap();
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
            sequence: self.sequence,
        })
    }

    pub fn verify_preflight_check(&self) -> Result<()> {
        for sequence in self.sequence.sequences() {
            if self.args.verifier.verifier == VerificationProviderType::Etherscan &&
                self.script_config
                    .config
                    .get_etherscan_api_key(Some(sequence.chain.into()))
                    .is_none()
            {
                eyre::bail!("Missing etherscan key for chain {}", sequence.chain);
            }
        }

        Ok(())
    }
}
