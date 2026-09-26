use std::{cmp::Ordering, num::NonZeroU64, sync::Arc, time::Duration};

use crate::{
    ScriptArgs, ScriptConfig,
    build::LinkedBuildData,
    progress::ScriptProgress,
    sequence::ScriptSequenceKind,
    session::{
        RemainingScriptTransaction, SignerScope,
        insert_session_access_key_for_remaining_transactions,
        script_session_expected_sender_if_configured,
    },
    verify::BroadcastedState,
};
use alloy_chains::{Chain, NamedChain};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_network::{
    EthereumWallet, Network, NetworkTransactionBuilder, ReceiptResponse, TransactionBuilder,
};
use alloy_primitives::{
    Address, Bytes, TxHash, TxKind, U256, keccak256,
    map::{AddressHashMap, AddressHashSet, HashMap},
    utils::format_units,
};
use alloy_provider::{Provider, RootProvider, utils::Eip1559Estimation};
use alloy_rpc_types::TransactionRequest;
use alloy_signer::Signature;
use eyre::{Context, Result, bail};
use forge_script_sequence::ScriptSequence;
use foundry_cheatcodes::Wallets;
use foundry_cli::utils::{has_batch_support, has_different_gas_calc};
use foundry_common::{
    FoundryTransactionBuilder, TransactionMaybeSigned,
    provider::{
        ProviderBuilder,
        fee::{estimate_eip1559_fees, resolve_broadcast_eip1559_fees},
    },
    shell,
    tempo::{TempoSponsor, maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_config::Config;
use foundry_evm::core::{
    constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH,
    evm::{FoundryEvmNetwork, TempoEvmNetwork},
    fork::ResolvedFork,
    opts::EvmOpts,
};
use foundry_wallets::{TempoAccountsWallet, wallet_browser::signer::BrowserSigner};
use futures::{FutureExt, StreamExt, future::join_all, stream::FuturesUnordered};
use itertools::Itertools;
use revm_inspectors::tracing::types::CallKind;
use tempo_alloy::{TempoNetwork, rpc::TempoTransactionRequest};
use tempo_primitives::transaction::Call;

/// Represents how to send a single transaction.
#[derive(Clone)]
pub enum SendTransactionKind<'a, N: Network> {
    Unlocked(N::TransactionRequest),
    Raw(N::TransactionRequest, &'a EthereumWallet),
    Browser(N::TransactionRequest, &'a BrowserSigner<N>),
    Signed(N::TxEnvelope),
    AccessKey(N::TransactionRequest, Box<TempoAccountsWallet>),
    PreparedRaw(Bytes, TxHash),
}

impl<'a, N: Network> SendTransactionKind<'a, N>
where
    N::TxEnvelope: From<Signed<N::UnsignedTx>>,
    N::UnsignedTx: SignableTransaction<Signature>,
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    const fn is_local(&self) -> bool {
        matches!(
            self,
            Self::Raw(..) | Self::Signed(_) | Self::AccessKey(..) | Self::PreparedRaw(..)
        )
    }

    /// Prepares the transaction for broadcasting by synchronizing nonce and estimating gas.
    ///
    /// This method performs two key operations:
    /// 1. Nonce synchronization: Waits for the provider's nonce to catch up to the expected
    ///    transaction nonce when doing sequential broadcast
    /// 2. Gas estimation: Re-estimates gas right before broadcasting for chains that require it
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare(
        &mut self,
        provider: &RootProvider<N>,
        sequential_broadcast: bool,
        is_fixed_gas_limit: bool,
        estimate_via_rpc: bool,
        estimate_multiplier: u64,
        tempo_sponsor: Option<&TempoSponsor>,
        chain: Option<Chain>,
    ) -> Result<()> {
        let tempo_browser = matches!(self, Self::Browser(..)) && chain.is_some_and(Chain::is_tempo);
        let (tx, tempo_wallet) = match self {
            Self::Raw(tx, _) | Self::Unlocked(tx) | Self::Browser(tx, _) => (tx, None),
            Self::AccessKey(tx, wallet) => (tx, Some(wallet)),
            Self::Signed(_) | Self::PreparedRaw(..) => return Ok(()),
        };

        reject_access_key_create::<N>(tx, tempo_wallet.is_some())?;

        if sequential_broadcast {
            let from = tx.from().expect("no sender");

            let tx_nonce = tx.nonce().expect("no nonce");
            for attempt in 0..5 {
                let nonce = provider.get_transaction_count(from).await?;
                match nonce.cmp(&tx_nonce) {
                    Ordering::Greater => {
                        bail!(
                            "EOA nonce changed unexpectedly while sending transactions. Expected {tx_nonce} got {nonce} from provider."
                        );
                    }
                    Ordering::Less => {
                        if attempt == 4 {
                            bail!(
                                "After 5 attempts, provider nonce ({nonce}) is still behind expected nonce ({tx_nonce})."
                            );
                        }
                        warn!(
                            "Expected nonce ({tx_nonce}) is ahead of provider nonce ({nonce}). Retrying in 1 second..."
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    }
                    Ordering::Equal => {
                        // Nonces are equal, we can proceed.
                        break;
                    }
                }
            }
        }

        if let Some(wallet) = tempo_wallet {
            **wallet = tx.prepare_with_tempo_wallet(provider, wallet).await?;
        }

        let fee_token = if let Some(sponsor) = tempo_sponsor {
            sponsor.resolve_and_set_fee_token(Some(provider), chain, tx).await?;
            None
        } else {
            resolve_and_set_fee_token(Some(provider), chain, tx, tx.from()).await?
        };

        // A fee token, sponsor, validity window, or other Tempo field selects
        // the AA transaction type. AA requests carry CREATE as their first
        // call rather than as the Ethereum transaction `to` field.
        convert_tempo_aa_create::<N>(tx);

        // Chains which use `eth_estimateGas` are being sent sequentially and require their
        // gas to be re-estimated right before broadcasting.
        if !is_fixed_gas_limit && estimate_via_rpc {
            estimate_gas(tx, provider, estimate_multiplier, tempo_browser).await?;
        }

        if let Some(sponsor) = tempo_sponsor {
            let from = tx.from().expect("no sender");
            sponsor.attach_and_print::<N>(tx, from).await?;
        } else {
            maybe_print_fee_token(Some(provider), fee_token).await?;
        }

        Ok(())
    }

    /// Sends the transaction to the network.
    ///
    /// Depending on the transaction kind, this will either:
    /// - Submit via `eth_sendTransaction` for unlocked accounts
    /// - Sign and submit via `eth_sendRawTransaction` for raw transactions
    /// - Submit pre-signed transaction via `eth_sendRawTransaction`
    pub async fn send(self, provider: Arc<RootProvider<N>>) -> Result<TxHash> {
        match self {
            Self::Unlocked(tx) => {
                debug!("sending transaction from unlocked account {:?}", tx);

                // Submit the transaction
                let pending = provider.send_transaction(tx).await?;
                Ok(*pending.tx_hash())
            }
            Self::Raw(tx, signer) => {
                debug!("sending transaction: {:?}", tx);
                let signed = tx.build(signer).await?;

                // Submit the raw transaction
                let pending = provider.send_raw_transaction(signed.encoded_2718().as_ref()).await?;
                Ok(*pending.tx_hash())
            }
            Self::Signed(tx) => {
                debug!("sending transaction: {:?}", tx);
                let pending = provider.send_raw_transaction(tx.encoded_2718().as_ref()).await?;
                Ok(*pending.tx_hash())
            }
            Self::Browser(tx, signer) => {
                debug!("sending transaction: {:?}", tx);

                // Sign and send the transaction via the browser wallet
                Ok(signer.send_transaction_via_browser(tx).await?)
            }
            Self::AccessKey(tx, wallet) => {
                debug!("sending transaction via tempo access key: {:?}", tx);

                let raw_tx = tx.sign_with_tempo_wallet(&wallet).await?;

                let pending = provider.send_raw_transaction(&raw_tx).await?;
                Ok(*pending.tx_hash())
            }
            Self::PreparedRaw(payload, _) => {
                let pending = provider.send_raw_transaction(&payload).await?;
                Ok(*pending.tx_hash())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_for_durable_send(
        mut self,
        provider: &RootProvider<N>,
        sequential_broadcast: bool,
        is_fixed_gas_limit: bool,
        estimate_via_rpc: bool,
        estimate_multiplier: u64,
        tempo_sponsor: Option<&TempoSponsor>,
        chain: Option<Chain>,
    ) -> Result<Self> {
        self.prepare(
            provider,
            sequential_broadcast,
            is_fixed_gas_limit,
            estimate_via_rpc,
            estimate_multiplier,
            tempo_sponsor,
            chain,
        )
        .await?;
        Ok(match self {
            Self::Raw(tx, signer) => {
                let signed = tx.build(signer).await?;
                Self::PreparedRaw(Bytes::from(signed.encoded_2718()), signed.trie_hash())
            }
            Self::Signed(tx) => {
                let hash = tx.trie_hash();
                Self::PreparedRaw(Bytes::from(tx.encoded_2718()), hash)
            }
            Self::AccessKey(tx, wallet) => {
                let payload = Bytes::from(tx.sign_with_tempo_wallet(&wallet).await?);
                let envelope = N::TxEnvelope::decode_2718_exact(&payload)?;
                Self::PreparedRaw(payload, envelope.trie_hash())
            }
            kind => kind,
        })
    }

    /// Prepares and sends the transaction in one operation.
    ///
    /// This is a convenience method that combines [`prepare`](Self::prepare) and
    /// [`send`](Self::send) into a single call.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_and_send(
        mut self,
        provider: Arc<RootProvider<N>>,
        sequential_broadcast: bool,
        is_fixed_gas_limit: bool,
        estimate_via_rpc: bool,
        estimate_multiplier: u64,
        tempo_sponsor: Option<&TempoSponsor>,
        chain: Option<Chain>,
    ) -> Result<TxHash> {
        self.prepare(
            &provider,
            sequential_broadcast,
            is_fixed_gas_limit,
            estimate_via_rpc,
            estimate_multiplier,
            tempo_sponsor,
            chain,
        )
        .await?;

        self.send(provider).await
    }
}

#[cfg(test)]
pub(crate) fn remaining_unsigned_transactions<N: Network>(
    sequences: &[ScriptSequence<N>],
) -> impl Iterator<Item = RemainingScriptTransaction> + '_ {
    sequences.iter().flat_map(|sequence| {
        remaining_transactions(sequence).filter(|tx| tx.is_unsigned()).map(|tx| {
            RemainingScriptTransaction {
                chain: sequence.chain,
                from: tx.from().expect("missing from"),
            }
        })
    })
}

pub(crate) fn remaining_unsigned_transactions_for_recovery<N: Network>(
    sequence: &ScriptSequenceKind<N>,
) -> Vec<RemainingScriptTransaction>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
    N::TransactionRequest: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    sequence
        .sequences()
        .iter()
        .enumerate()
        .flat_map(|(sequence_index, deployment)| {
            remaining_operation_indices(sequence, sequence_index).into_iter().filter_map(
                move |index| {
                    let tx = deployment.transactions[index].tx();
                    (tx.is_unsigned() && sequence.signed_payload(sequence_index, index).is_none())
                        .then(|| RemainingScriptTransaction {
                            chain: deployment.chain,
                            from: tx.from().expect("missing from"),
                        })
                },
            )
        })
        .collect()
}

fn remaining_operation_indices<N: Network>(
    sequence: &ScriptSequenceKind<N>,
    sequence_index: usize,
) -> Vec<usize>
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
    N::TransactionRequest: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    let deployment = &sequence.sequences()[sequence_index];
    deployment
        .transactions
        .iter()
        .enumerate()
        .filter_map(|(index, transaction)| {
            let hash = sequence
                .signed_payload(sequence_index, index)
                .map(|signed| signed.hash)
                .or(transaction.hash)
                .or_else(|| match transaction.tx() {
                    TransactionMaybeSigned::Signed { tx, .. } => Some(tx.trie_hash()),
                    TransactionMaybeSigned::Unsigned(_) => None,
                });
            let completed = hash.is_some_and(|hash| {
                deployment.receipts.iter().any(|receipt| receipt.transaction_hash() == hash)
            });
            (!completed).then_some(index)
        })
        .collect()
}

fn remaining_sender_addresses<N: Network>(sequence: &ScriptSequenceKind<N>) -> AddressHashSet
where
    N::TxEnvelope: for<'de> serde::Deserialize<'de> + serde::Serialize,
    N::TransactionRequest: for<'de> serde::Deserialize<'de> + serde::Serialize,
{
    sequence
        .sequences()
        .iter()
        .enumerate()
        .flat_map(|(sequence_index, deployment)| {
            remaining_operation_indices(sequence, sequence_index)
                .into_iter()
                .filter_map(|index| deployment.transactions[index].tx().from())
        })
        .collect()
}

const fn should_broadcast_sequentially(
    estimate_via_rpc: bool,
    slow: bool,
    required_signers: usize,
    ordering_senders: usize,
    batch_supported: bool,
) -> bool {
    estimate_via_rpc || slow || required_signers == 0 || ordering_senders != 1 || !batch_supported
}

fn remaining_transaction_start<N: Network>(sequence: &ScriptSequence<N>) -> usize {
    sequence.receipts.len().min(sequence.transactions.len())
}

fn remaining_transactions<N: Network>(
    sequence: &ScriptSequence<N>,
) -> impl Iterator<Item = &TransactionMaybeSigned<N>> + '_ {
    sequence.transactions().skip(remaining_transaction_start(sequence))
}

/// Represents how to send _all_ transactions
pub enum SendTransactionsKind<N: Network> {
    /// Send via `eth_sendTransaction` and rely on the  `from` address being unlocked.
    Unlocked(AddressHashSet),
    /// Send a signed transaction via `eth_sendRawTransaction`, or via browser
    Raw {
        eth_wallets: AddressHashMap<EthereumWallet>,
        browser: Option<BrowserSigner<N>>,
        access_keys: HashMap<SignerScope, TempoAccountsWallet>,
    },
}

impl<N: Network> SendTransactionsKind<N> {
    /// Returns the [`SendTransactionKind`] for the given address
    ///
    /// Returns an error if no matching signer is found or the address is not unlocked
    pub fn for_sender(
        &self,
        chain: u64,
        addr: &Address,
        tx: N::TransactionRequest,
    ) -> Result<SendTransactionKind<'_, N>> {
        match self {
            Self::Unlocked(unlocked) => {
                if !unlocked.contains(addr) {
                    bail!("Sender address {:?} is not unlocked", addr);
                }
                Ok(SendTransactionKind::Unlocked(tx))
            }
            Self::Raw { eth_wallets, browser, access_keys } => {
                if let Some(wallet) = access_keys.get(&SignerScope::new(chain, *addr)) {
                    Ok(SendTransactionKind::AccessKey(tx, Box::new(wallet.clone())))
                } else if let Some(wallet) = eth_wallets.get(addr) {
                    Ok(SendTransactionKind::Raw(tx, wallet))
                } else if let Some(b) = browser
                    && b.address() == *addr
                {
                    Ok(SendTransactionKind::Browser(tx, b))
                } else {
                    bail!("No matching signer for {:?} found", addr);
                }
            }
        }
    }
}

/// State after we have bundled all
/// [`TransactionWithMetadata`](forge_script_sequence::TransactionWithMetadata) objects into a
/// single [`ScriptSequenceKind`] object containing one or more script sequences.
pub struct BundledState<FEN: FoundryEvmNetwork> {
    pub args: ScriptArgs,
    pub script_config: ScriptConfig<FEN>,
    pub script_wallets: Wallets,
    pub browser_wallet: Option<BrowserSigner<FEN::Network>>,
    pub build_data: LinkedBuildData,
    pub sequence: ScriptSequenceKind<FEN::Network>,
}

impl<FEN: FoundryEvmNetwork> BundledState<FEN> {
    pub async fn wait_for_pending(mut self) -> Result<Self> {
        let progress = ScriptProgress::default();
        let progress_ref = &progress;
        let config = &self.script_config.config;
        let durable_hashes = (0..self.sequence.sequences().len())
            .map(|sequence| self.sequence.submission_hashes(sequence))
            .collect::<Vec<_>>();
        let futs = self
            .sequence
            .sequences_mut()
            .iter_mut()
            .zip(durable_hashes)
            .enumerate()
            .map(|(sequence_idx, (sequence, durable_hashes))| async move {
                let rpc_url = sequence.rpc_url();
                let provider =
                    Arc::new(ProviderBuilder::from_config_with_url(config, rpc_url)?.build()?);
                progress_ref
                    .wait_for_pending(
                        sequence_idx,
                        sequence,
                        &provider,
                        self.script_config.config.transaction_timeout,
                        self.args.confirmations,
                        (&durable_hashes, &durable_hashes),
                    )
                    .await
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
    pub async fn broadcast(mut self) -> Result<BroadcastedState<FEN>>
    where
        <FEN::Network as Network>::TxEnvelope: alloy_consensus::transaction::SignerRecoverable,
    {
        let remaining_transactions = remaining_unsigned_transactions_for_recovery(&self.sequence);
        let ordering_addresses = remaining_sender_addresses(&self.sequence);
        let has_unprepared_transactions =
            self.sequence.sequences().iter().enumerate().any(|(sequence_index, _)| {
                remaining_operation_indices(&self.sequence, sequence_index)
                    .into_iter()
                    .any(|index| self.sequence.signed_payload(sequence_index, index).is_none())
            });
        let required_addresses =
            remaining_transactions.iter().map(|tx| tx.from).collect::<AddressHashSet>();

        if required_addresses.contains(&Config::DEFAULT_SENDER) {
            eyre::bail!(
                "You seem to be using Foundry's default sender. Be sure to set your own --sender."
            );
        }

        let send_kind = if required_addresses.is_empty() {
            SendTransactionsKind::Raw {
                eth_wallets: AddressHashMap::default(),
                browser: None,
                access_keys: HashMap::default(),
            }
        } else if self.args.unlocked {
            SendTransactionsKind::Unlocked(required_addresses.clone())
        } else {
            let expected_session_sender = script_session_expected_sender_if_configured(
                &self.script_config.tempo,
                &required_addresses,
            )?;

            // For addresses without an explicit signer, try the Tempo Accounts store.
            let mut access_keys: HashMap<SignerScope, TempoAccountsWallet> = HashMap::default();
            if let Some(expected_session_sender) = expected_session_sender
                && let Some(session) =
                    self.script_config.tempo.session_signer_for_multi_wallet_any_chain(
                        &self.args.wallets,
                        Some(expected_session_sender),
                    )?
            {
                insert_session_access_key_for_remaining_transactions(
                    &mut access_keys,
                    session,
                    &remaining_transactions,
                )?;
            }

            let signers: Vec<Address> = self
                .script_wallets
                .signers()
                .map_err(|e| eyre::eyre!("{e}"))?
                .into_iter()
                .chain(self.browser_wallet.as_ref().map(|b| b.address()))
                .collect();

            let mut missing_addresses = Vec::new();
            let accounts_wallet = self
                .script_config
                .evm_opts
                .networks
                .is_tempo()
                .then(TempoAccountsWallet::try_from_default_store)
                .transpose()?
                .flatten();

            for tx in &remaining_transactions {
                let scope = tx.scope();
                if !signers.contains(&tx.from) && !access_keys.contains_key(&scope) {
                    if let Some(wallet) = accounts_wallet.as_ref()
                        && wallet.has_account(tx.from)?
                    {
                        access_keys.insert(scope, wallet.clone().with_chain_id(tx.chain));
                    } else {
                        missing_addresses.push(tx.from);
                    }
                }
            }

            missing_addresses.sort_unstable();
            missing_addresses.dedup();

            if !missing_addresses.is_empty() {
                eyre::bail!(
                    "No associated wallet for addresses: {:?}. Unlocked wallets: {:?}",
                    missing_addresses,
                    signers
                );
            }

            let signers = self.script_wallets.into_multi_wallet().into_signers()?;
            let eth_wallets: AddressHashMap<EthereumWallet> =
                signers.into_iter().map(|(addr, signer)| (addr, signer.into())).collect();

            SendTransactionsKind::Raw { eth_wallets, browser: self.browser_wallet, access_keys }
        };

        let tempo_sponsor = if has_unprepared_transactions {
            self.script_config.tempo.sponsor_config().await?.map(Arc::new)
        } else {
            None
        };
        if tempo_sponsor.is_some()
            && self.script_config.tempo.sponsor_sig.is_some()
            && remaining_transactions.len() > 1
        {
            eyre::bail!(
                "--tempo.sponsor-sig can only sponsor one remaining script transaction; use --tempo.sponsor-signer for multi-transaction scripts"
            );
        }

        let progress = ScriptProgress::default();

        for i in 0..self.sequence.sequences().len() {
            let remaining_indices = remaining_operation_indices(&self.sequence, i);
            let signed_payloads = (0..self.sequence.sequences()[i].transactions.len())
                .map(|index| {
                    self.sequence
                        .signed_payload(i, index)
                        .map(|signed| (signed.payload.clone(), signed.hash))
                })
                .collect::<Vec<_>>();
            let durable_hashes = signed_payloads
                .iter()
                .filter_map(|signed| signed.as_ref().map(|(_, hash)| *hash))
                .collect::<Vec<_>>();
            let mut sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

            let provider = Arc::new(
                ProviderBuilder::from_config_with_url(
                    &self.script_config.config,
                    sequence.rpc_url(),
                )?
                .build()?,
            );

            let seq_progress = progress.get_sequence_progress(i, sequence);

            if !remaining_indices.is_empty() {
                let is_legacy = Chain::from(sequence.chain).is_legacy() || self.args.legacy;
                // Make a one-time gas price estimation
                let all_signed =
                    remaining_indices.iter().all(|index| signed_payloads[*index].is_some());
                let (gas_price, eip1559_fees) = match (
                    all_signed,
                    is_legacy,
                    self.args.with_gas_price,
                    self.args.priority_gas_price,
                ) {
                    (true, _, _, _) => (None, None),
                    (false, true, Some(gas_price), _) => (Some(gas_price.to()), None),
                    (false, true, None, _) => (Some(provider.get_gas_price().await?), None),
                    (false, false, Some(max_fee_per_gas), Some(max_priority_fee_per_gas)) => {
                        let max_fee: u128 = max_fee_per_gas.to();
                        let max_priority: u128 = max_priority_fee_per_gas.to();
                        if max_priority > max_fee {
                            eyre::bail!(
                                "--priority-gas-price ({max_priority}) cannot be higher than --with-gas-price ({max_fee})"
                            );
                        }
                        (
                            None,
                            Some(Eip1559Estimation {
                                max_fee_per_gas: max_fee,
                                max_priority_fee_per_gas: max_priority,
                            }),
                        )
                    }
                    (false, false, _, _) => {
                        let fees = estimate_eip1559_fees(
                            &provider,
                            self.script_config.config.eip1559_fee_estimate,
                        )
                        .await
                        .wrap_err("Failed to estimate EIP1559 fees. This chain might not support EIP1559, try adding --legacy to your command.")?;

                        // Browser wallets may suggest their own tip; query it best-effort.
                        let browser_suggested_tip = if matches!(
                            &send_kind,
                            SendTransactionsKind::Raw { browser: Some(_), .. }
                        ) {
                            provider.get_max_priority_fee_per_gas().await.ok()
                        } else {
                            None
                        };

                        let fees = resolve_broadcast_eip1559_fees(
                            fees,
                            self.args.with_gas_price.map(|p| p.to()),
                            self.args.priority_gas_price.map(|p| p.to()),
                            browser_suggested_tip,
                        )?;

                        (None, Some(fees.estimation()))
                    }
                };

                // Iterate through transactions, matching the `from` field with the associated
                // wallet. Then send the transaction. Panics if we find a unknown `from`
                let sequence_chain = sequence.chain;
                let mut transactions = Vec::with_capacity(remaining_indices.len());
                for index in remaining_indices {
                    let tx_with_metadata = &sequence.transactions[index];
                    let is_fixed_gas_limit = tx_with_metadata.is_fixed_gas_limit;

                    let kind = match (&signed_payloads[index], tx_with_metadata.tx().clone()) {
                        (Some((payload, hash)), _) => {
                            SendTransactionKind::PreparedRaw(payload.clone(), *hash)
                        }
                        (None, TransactionMaybeSigned::Signed { tx, .. }) => {
                            if tempo_sponsor.is_some() {
                                eyre::bail!(
                                    "cannot attach Tempo sponsor signature to an already signed script transaction"
                                );
                            }
                            SendTransactionKind::Signed(tx)
                        }
                        (None, TransactionMaybeSigned::Unsigned(mut tx)) => {
                            let from = tx.from().expect("No sender for onchain transaction!");

                            tx.set_chain_id(sequence_chain);

                            // Set TxKind::Create explicitly to satisfy `check_reqd_fields` in
                            // alloy
                            if tx.kind().is_none() {
                                tx.set_create();
                            }

                            if let Some(gas_price) = gas_price {
                                tx.set_gas_price(gas_price);
                            } else {
                                let eip1559_fees = eip1559_fees.expect("was set above");
                                tx.set_max_priority_fee_per_gas(
                                    eip1559_fees.max_priority_fee_per_gas,
                                );
                                tx.set_max_fee_per_gas(eip1559_fees.max_fee_per_gas);
                            }

                            self.script_config.tempo.apply::<FEN::Network>(&mut tx, None);

                            send_kind.for_sender(sequence_chain, &from, tx)?
                        }
                    };
                    transactions.push((kind, is_fixed_gas_limit, index));
                }

                let estimate_via_rpc = has_different_gas_calc(sequence.chain)
                    || self.script_config.evm_opts.networks.is_tempo()
                    || self.args.skip_simulation;

                // We only wait for a transaction receipt before sending the next transaction, if
                // there is more than one signer. There would be no way of assuring
                // their order otherwise.
                // Or if the chain does not support batched transactions (eg. Arbitrum).
                // Or if we need to invoke eth_estimateGas before sending transactions.
                let sequential_broadcast = should_broadcast_sequentially(
                    estimate_via_rpc,
                    self.args.slow,
                    required_addresses.len(),
                    ordering_addresses.len(),
                    has_batch_support(sequence.chain),
                );

                // We send transactions and wait for receipts in batches of 100, since some networks
                // cannot handle more than that.
                let batch_size = if sequential_broadcast { 1 } else { 100 };
                let sequence_chain = sequence.chain;

                for (batch_number, batch) in transactions.chunks(batch_size).enumerate() {
                    seq_progress.inner.write().set_status(&format!(
                        "Sending transactions [{} - {}]",
                        batch_number * batch_size,
                        batch_number * batch_size + std::cmp::min(batch_size, batch.len()) - 1
                    ));

                    if !batch.is_empty() {
                        let mut prepared = Vec::with_capacity(batch.len());
                        for (kind, is_fixed_gas_limit, index) in batch {
                            let mut kind = if kind.is_local() {
                                kind.clone()
                                    .prepare_for_durable_send(
                                        &provider,
                                        sequential_broadcast,
                                        *is_fixed_gas_limit,
                                        estimate_via_rpc,
                                        self.args.gas_estimate_multiplier,
                                        tempo_sponsor.as_deref(),
                                        Some(sequence_chain.into()),
                                    )
                                    .await?
                            } else {
                                kind.clone()
                            };
                            if let SendTransactionKind::PreparedRaw(payload, hash) = &mut kind {
                                *hash = self.sequence.persist_signed_payload(
                                    i,
                                    *index,
                                    payload.clone(),
                                )?;
                            }
                            prepared.push((kind, *is_fixed_gas_limit, *index));
                        }
                        sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

                        let pending_transactions =
                            prepared.iter().map(|(kind, is_fixed_gas_limit, index)| {
                                let provider = provider.clone();
                                let tempo_sponsor = tempo_sponsor.clone();
                                async move {
                                    let res = kind
                                        .clone()
                                        .prepare_and_send(
                                            provider,
                                            sequential_broadcast,
                                            *is_fixed_gas_limit,
                                            estimate_via_rpc,
                                            self.args.gas_estimate_multiplier,
                                            tempo_sponsor.as_deref(),
                                            Some(sequence_chain.into()),
                                        )
                                        .await;
                                    (res, kind, *is_fixed_gas_limit, 0, None, *index)
                                }
                                .boxed()
                            });

                        let mut buffer = pending_transactions.collect::<FuturesUnordered<_>>();

                        'send: while let Some((
                            mut res,
                            kind,
                            is_fixed_gas_limit,
                            attempt,
                            original_res,
                            index,
                        )) = buffer.next().await
                        {
                            if res.is_err()
                                && let SendTransactionKind::PreparedRaw(_, hash) = kind
                                && provider
                                    .get_transaction_by_hash(*hash)
                                    .await
                                    .is_ok_and(|transaction| transaction.is_some())
                            {
                                res = Ok(*hash);
                            }
                            if res.is_err()
                                && self.script_config.tempo.sponsor_sig.is_some()
                                && !matches!(kind, SendTransactionKind::PreparedRaw(..))
                                && attempt == 0
                            {
                                debug!(
                                    "not retrying transaction because --tempo.sponsor-sig is a static signature"
                                );
                            } else if res.is_err() && attempt <= 3 {
                                // Try to resubmit the transaction
                                let provider = provider.clone();
                                let progress = seq_progress.inner.clone();
                                let tempo_sponsor = tempo_sponsor.clone();
                                buffer.push(Box::pin(async move {
                                    debug!(err=?res, ?attempt, "retrying transaction ");
                                    let attempt = attempt + 1;
                                    progress.write().set_status(&format!(
                                        "retrying transaction {res:?} (attempt {attempt})"
                                    ));
                                    tokio::time::sleep(Duration::from_millis(1000 * attempt)).await;
                                    let r = kind
                                        .clone()
                                        .prepare_and_send(
                                            provider,
                                            sequential_broadcast,
                                            is_fixed_gas_limit,
                                            estimate_via_rpc,
                                            self.args.gas_estimate_multiplier,
                                            tempo_sponsor.as_deref(),
                                            Some(sequence_chain.into()),
                                        )
                                        .await;
                                    (
                                        r,
                                        kind,
                                        is_fixed_gas_limit,
                                        attempt,
                                        original_res.or(Some(res)),
                                        index,
                                    )
                                }));

                                continue 'send;
                            }

                            // Preserve the original error if any
                            let tx_hash = res.wrap_err_with(|| {
                                if let Some(original_res) = original_res {
                                    format!(
                                        "Failed to send transaction after {attempt} attempts {original_res:?}"
                                    )
                                } else {
                                    "Failed to send transaction".to_string()
                                }
                            })?;
                            if let SendTransactionKind::PreparedRaw(_, expected) = kind
                                && *expected != tx_hash
                            {
                                bail!("RPC returned hash {tx_hash} for signed payload {expected}");
                            }
                            sequence.add_pending(index, tx_hash);

                            // Checkpoint save
                            self.sequence.save(true, false)?;
                            sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

                            seq_progress.inner.write().tx_sent(tx_hash);
                        }

                        // Checkpoint save
                        self.sequence.save(true, false)?;
                        sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

                        progress
                            .wait_for_pending(
                                i,
                                sequence,
                                &provider,
                                self.script_config.config.transaction_timeout,
                                self.args.confirmations,
                                (&durable_hashes, &durable_hashes),
                            )
                            .await?
                    }
                    // Checkpoint save
                    self.sequence.save(true, false)?;
                    sequence = self.sequence.sequences_mut().get_mut(i).unwrap();
                }
            }

            let (total_gas, total_gas_price, total_paid) =
                sequence.receipts.iter().fold((0, 0, 0), |acc, receipt| {
                    let gas_used = receipt.gas_used();
                    let gas_price = receipt.effective_gas_price() as u64;
                    (acc.0 + gas_used, acc.1 + gas_price, acc.2 + gas_used * gas_price)
                });
            let paid = format_units(total_paid, 18).unwrap_or_else(|_| "N/A".to_string());
            let avg_gas_price = total_gas_price
                .checked_div(sequence.receipts.len() as u64)
                .and_then(|avg| format_units(avg, 9).ok())
                .unwrap_or_else(|| "N/A".to_string());

            let token_symbol = NamedChain::try_from(sequence.chain)
                .unwrap_or_default()
                .native_currency_symbol()
                .unwrap_or("ETH");
            seq_progress.inner.write().set_status(&format!(
                "Total Paid: {} {} ({} gas * avg {} gwei)\n",
                paid.trim_end_matches('0'),
                token_symbol,
                total_gas,
                avg_gas_price.trim_end_matches('0').trim_end_matches('.')
            ));
            seq_progress.inner.write().finish();
        }

        if !shell::is_json() {
            sh_println!("\n\n==========================")?;
            sh_println!("\nONCHAIN EXECUTION COMPLETE & SUCCESSFUL.")?;
        }

        Ok(BroadcastedState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            sequence: self.sequence,
        })
    }

    pub async fn verify_preflight_check(&self) -> Result<()> {
        if self.args.verify_external && self.script_config.config.offline {
            bail!("External contract verification is unavailable in offline mode");
        }

        for sequence in self.sequence.sequences() {
            let chain: Chain = sequence.chain.into();
            // Resolve the API key: CLI arg first, then per-chain config, then global fallback.
            let etherscan_key = self
                .script_config
                .config
                .get_etherscan_api_key(Some(chain))
                .or_else(|| self.script_config.config.etherscan_api_key.clone());
            let api_key =
                self.args.verifier.resolve_api_key(etherscan_key.as_deref()).map(str::to_owned);
            let has_url = self.args.verifier.verifier_url.is_some();
            let is_explicit = self.args.verifier.is_explicitly_set();
            // Presence check: use the fully-resolved provider type so that implicit Etherscan
            // selection (key from env/config, no explicit --verifier flag) is validated too.
            self.args
                .verifier
                .resolve(api_key.as_deref(), Some(chain))
                .client(api_key.as_deref(), Some(chain), has_url, is_explicit)
                .wrap_err_with(|| {
                    format!("Verification preflight check failed for chain {}", sequence.chain)
                })?;
            // Connectivity check: validates credentials are actually accepted by the verifier.
            self.args
                .verifier
                .check_credentials(api_key.as_deref(), chain, &self.script_config.config)
                .await
                .wrap_err_with(|| {
                    format!("Verification preflight check failed for chain {}", sequence.chain)
                })?;
        }

        Ok(())
    }
}

impl BundledState<TempoEvmNetwork> {
    /// Broadcasts all transactions as a single Tempo batch transaction (type 0x76).
    ///
    /// This method collects all individual transactions from the script and combines them
    /// into a single batch transaction for atomic execution on Tempo.
    pub async fn broadcast_batch(mut self) -> Result<BroadcastedState<TempoEvmNetwork>> {
        // Batch mode only supports single chain for now
        if self.sequence.sequences().len() != 1 {
            bail!(
                "--batch mode only supports single-chain scripts. \
                 Use --multi without --batch for multi-chain."
            );
        }

        let sequence = self.sequence.sequences_mut().get_mut(0).unwrap();
        let total_transactions = sequence.transactions.len();
        let remaining_start = remaining_transaction_start(sequence);

        if remaining_start == total_transactions {
            sh_println!("No transactions to broadcast in batch mode.")?;
            return Ok(BroadcastedState {
                args: self.args,
                script_config: self.script_config,
                build_data: self.build_data,
                sequence: self.sequence,
            });
        }

        // Reject pre-signed transactions: a batch is a single atomic tx from one sender,
        // so any tx already signed by another key would silently be re-attributed.
        if let Some((idx, _)) =
            sequence.transactions().enumerate().find(|(_, tx)| !tx.is_unsigned())
        {
            bail!(
                "--batch cannot include pre-signed transactions (found at position {}); \
                 batch mode signs a single atomic transaction from one sender.",
                idx + 1
            );
        }

        // Collect sender addresses - batch mode requires single sender
        let senders: AddressHashSet = remaining_transactions(sequence)
            .filter(|tx| tx.is_unsigned())
            .filter_map(|tx| tx.from())
            .collect();

        if senders.len() != 1 {
            bail!(
                "--batch mode requires all transactions to have the same sender. \
                 Found {} unique senders: {:?}",
                senders.len(),
                senders
            );
        }

        let sender = *senders.iter().next().unwrap();
        let chain_id = sequence.chain;

        if sender == Config::DEFAULT_SENDER {
            bail!(
                "You seem to be using Foundry's default sender. Be sure to set your own --sender."
            );
        }

        let provider = Arc::new(
            ProviderBuilder::<TempoNetwork>::from_config_with_url(
                &self.script_config.config,
                sequence.rpc_url(),
            )?
            .build()?,
        );

        // Resume detection happens before signer resolution, gas estimation, and sponsor attachment
        // so that recovering an already-submitted batch tx never requires the original
        // signer/sponsor or a fresh estimate.
        //
        // If the hash is found in the stamped transactions but a receipt cannot be obtained within
        // the timeout, the tx is assumed dropped. We clear the stamped hashes so that a subsequent
        // --resume will re-send a replacement instead of waiting on a dead hash.
        let pending_batch_hash: Option<TxHash> =
            sequence.transactions.iter().skip(remaining_start).find_map(|tx| tx.hash);

        if let Some(tx_hash) = pending_batch_hash {
            sh_println!(
                "Resuming batch: tx {tx_hash:#x} already submitted, waiting for receipt..."
            )?;

            let timeout = self.script_config.config.transaction_timeout;
            let receipt_result = tokio::time::timeout(
                Duration::from_secs(timeout),
                wait_for_batch_receipt(provider.as_ref(), tx_hash, self.args.confirmations),
            )
            .await;

            match receipt_result {
                Ok(Ok(Some(receipt))) => {
                    // Tx confirmed, process receipt and return without touching signer/sponsor.
                    let success = receipt.status();
                    if success {
                        sh_println!(
                            "Batch transaction confirmed in block {}",
                            receipt.block_number.unwrap_or(0)
                        )?;
                    } else {
                        bail!("Batch transaction failed (reverted)");
                    }

                    let sequence = self.sequence.sequences_mut().get_mut(0).unwrap();
                    let remaining_len = sequence.transactions.len() - remaining_start;
                    let per_tx_addresses: Vec<Option<Address>> = sequence
                        .transactions
                        .iter()
                        .skip(remaining_start)
                        .map(|tx| match tx.call_kind {
                            CallKind::Create | CallKind::Create2 => tx.contract_address,
                            _ => None,
                        })
                        .collect();

                    for (idx, addr) in per_tx_addresses.iter().enumerate() {
                        if let Some(addr) = addr {
                            sh_println!("  call[{idx}] deployed at: {addr:#x}")?;
                        }
                    }

                    for addr in &per_tx_addresses {
                        let mut tx_receipt = receipt.clone();
                        tx_receipt.contract_address = *addr;
                        sequence.receipts.push(tx_receipt);
                    }
                    // Clear the pending entry now that we have a receipt.
                    sequence.remove_pending(tx_hash);

                    let chain = sequence.chain;
                    let _ = sequence;
                    self.sequence.save(true, false)?;

                    let total_gas = receipt.gas_used();
                    let gas_price = receipt.effective_gas_price() as u64;
                    let total_paid = total_gas * gas_price;
                    let paid = format_units(total_paid, 18).unwrap_or_else(|_| "N/A".to_string());
                    let gas_price_gwei =
                        format_units(gas_price, 9).unwrap_or_else(|_| "N/A".to_string());
                    let token_symbol = NamedChain::try_from(chain)
                        .unwrap_or_default()
                        .native_currency_symbol()
                        .unwrap_or("ETH");
                    sh_println!(
                        "\nTotal Paid: {} {} ({} gas * {} gwei)\n(resumed from previous run, {} tx(s))",
                        paid.trim_end_matches('0'),
                        token_symbol,
                        total_gas,
                        gas_price_gwei.trim_end_matches('0').trim_end_matches('.'),
                        remaining_len,
                    )?;

                    if !shell::is_json() {
                        sh_println!("\n\n==========================")?;
                        sh_println!("\nBATCH EXECUTION COMPLETE & SUCCESSFUL.")?;
                        sh_println!(
                            "All {} calls executed atomically in a single transaction.",
                            remaining_len
                        )?;
                    }

                    return Ok(BroadcastedState {
                        args: self.args,
                        script_config: self.script_config,
                        build_data: self.build_data,
                        sequence: self.sequence,
                    });
                }
                Ok(Ok(None)) => {
                    // Dropped from mempool, clear stamped hashes so the next --resume re-sends.
                    sh_println!(
                        "Batch tx {tx_hash:#x} was dropped from the mempool; will re-send..."
                    )?;
                    let sequence = self.sequence.sequences_mut().get_mut(0).unwrap();
                    sequence.remove_pending(tx_hash);
                    for tx in sequence.transactions.iter_mut().skip(remaining_start) {
                        tx.hash = None;
                    }
                    self.sequence.save(true, false)?;
                    // Fall through to full send path below.
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    // Timeout, clear stamped hashes so the next --resume can re-send rather than
                    // waiting indefinitely on a potentially dead hash.
                    sh_println!(
                        "Timeout waiting for batch tx {tx_hash:#x}; clearing checkpoint so \
                         --resume can re-send a replacement."
                    )?;
                    let sequence = self.sequence.sequences_mut().get_mut(0).unwrap();
                    sequence.remove_pending(tx_hash);
                    for tx in sequence.transactions.iter_mut().skip(remaining_start) {
                        tx.hash = None;
                    }
                    self.sequence.save(true, false)?;
                    return Err(eyre::eyre!(
                        "Timeout waiting for batch transaction receipt (tx: {tx_hash:#x}). \
                         The transaction hash has been cleared; run with --resume to retry."
                    ));
                }
            }
        }

        // Reborrow after the potential save above.
        let sequence = self.sequence.sequences_mut().get_mut(0).unwrap();

        let tempo_sponsor = self.script_config.tempo.sponsor_config().await?;

        // Get wallet for signing
        enum BatchSigner {
            Unlocked,
            Wallet(EthereumWallet),
            TempoKeychain(Box<TempoAccountsWallet>),
        }

        let mut batch_signer = if self.args.unlocked {
            BatchSigner::Unlocked
        } else if let Some(session) = self.script_config.tempo.session_signer_for_multi_wallet(
            &self.args.wallets,
            Some(sender),
            chain_id,
        )? {
            BatchSigner::TempoKeychain(Box::new(session.access_key))
        } else {
            let mut signers = self.script_wallets.into_multi_wallet().into_signers()?;
            if let Some(signer) = signers.remove(&sender) {
                BatchSigner::Wallet(EthereumWallet::new(signer))
            } else {
                // Try the Tempo Accounts store only for Tempo broadcasts.
                if self.script_config.evm_opts.networks.is_tempo()
                    && let Some(wallet) = TempoAccountsWallet::try_from_default_store()?
                    && wallet.has_account(sender)?
                {
                    BatchSigner::TempoKeychain(Box::new(wallet.with_chain_id(chain_id)))
                } else {
                    bail!("No wallet found for sender {}", sender);
                }
            }
        };

        let create2_deployer = self.script_config.evm_opts.create2_deployer;
        let mut calls: Vec<Call> = Vec::new();
        for (call_index, tx) in remaining_transactions(sequence).enumerate() {
            // --batch cannot carry EIP-7702 authorization lists: they require per-tx signing
            // and cannot be atomically bundled into a Tempo batch.
            if tx.authorization_list().is_some_and(|l| !l.is_empty()) {
                bail!(
                    "--batch does not support EIP-7702 authorization lists \
                     (found at transaction {}); use regular broadcast instead.",
                    call_index + 1
                );
            }
            // --batch cannot carry blob sidecars: Tempo batch txs are not blob-carrying txs.
            if let TransactionMaybeSigned::Unsigned(inner) = tx
                && inner.blob_sidecar().is_some()
            {
                bail!(
                    "--batch does not support blob (EIP-4844) transactions \
                     (found at transaction {}); use regular broadcast instead.",
                    call_index + 1
                );
            }

            // CREATEs are rewritten to CREATE2 via the Arachnid factory by the batch
            // inspector before broadcast, so tx.to() should always be Some here.
            let to = match tx.to() {
                Some(addr) => TxKind::Call(addr),
                None => {
                    bail!(
                        "Unexpected raw CREATE in --batch mode at position {} — \
                     this is a bug; CREATEs should have been rewritten by the inspector.",
                        call_index + 1
                    );
                }
            };
            let value = tx.value().unwrap_or(U256::ZERO);
            let input = tx.input().cloned().unwrap_or_default();

            calls.push(Call { to, value, input });
        }

        if calls.is_empty() {
            sh_println!("No transactions to broadcast in batch mode.")?;
            return Ok(BroadcastedState {
                args: self.args,
                script_config: self.script_config,
                build_data: self.build_data,
                sequence: self.sequence,
            });
        }

        // CREATE2 deployer must exist on-chain for any rewritten CREATEs.
        let needs_factory = sequence
            .transactions
            .iter()
            .skip(remaining_start)
            .any(|tx| matches!(tx.call_kind, CallKind::Create | CallKind::Create2));
        if needs_factory {
            let code = provider.get_code_at(create2_deployer).await?;
            if keccak256(&code) != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
                bail!(
                    "CREATE2 deployer {create2_deployer:#x} is not deployed on this Tempo network; \
                     --batch requires it. Deploy it first and retry."
                );
            }
        }

        sh_println!(
            "\n## Broadcasting batch transaction with {} call(s) to chain {}...",
            calls.len(),
            sequence.chain
        )?;

        // Build the batch transaction request
        let nonce = provider.get_transaction_count(sender).await?;

        // Batch transactions are Tempo-only and always use EIP-1559 style fees.
        let fees = estimate_eip1559_fees(&provider, self.script_config.config.eip1559_fee_estimate)
            .await?;
        let fees = resolve_broadcast_eip1559_fees(
            fees,
            self.args.with_gas_price.map(|p| p.to()),
            self.args.priority_gas_price.map(|p| p.to()),
            None,
        )?;
        let max_fee_per_gas = fees.max_fee_per_gas;
        let max_priority_fee_per_gas = fees.max_priority_fee_per_gas;

        let mut batch_tx = TempoTransactionRequest {
            inner: TransactionRequest {
                from: Some(sender),
                to: None,
                value: None,
                input: Default::default(),
                nonce: Some(nonce),
                chain_id: Some(chain_id),
                max_fee_per_gas: Some(max_fee_per_gas),
                max_priority_fee_per_gas: Some(max_priority_fee_per_gas),
                ..Default::default()
            },
            fee_token: self.script_config.tempo.fee_token,
            calls: calls.clone(),
            nonce_key: self.script_config.tempo.expiring_nonce.then_some(U256::MAX),
            valid_before: self.script_config.tempo.valid_before.and_then(NonZeroU64::new),
            ..Default::default()
        };
        self.script_config.tempo.apply::<TempoNetwork>(&mut batch_tx, None);
        let fee_token = if let Some(sponsor) = &tempo_sponsor {
            sponsor
                .resolve_and_set_fee_token(
                    Some(provider.as_ref()),
                    Some(Chain::from_named(NamedChain::Tempo)),
                    &mut batch_tx,
                )
                .await?;
            None
        } else {
            resolve_and_set_fee_token(
                Some(provider.as_ref()),
                Some(Chain::from_named(NamedChain::Tempo)),
                &mut batch_tx,
                Some(sender),
            )
            .await?
        };

        if let BatchSigner::TempoKeychain(wallet) = &mut batch_signer {
            **wallet = batch_tx.prepare_with_tempo_wallet(provider.as_ref(), wallet).await?;
        }

        // Estimate gas for the batch transaction
        estimate_gas(&mut batch_tx, provider.as_ref(), self.args.gas_estimate_multiplier, false)
            .await?;

        sh_println!("Estimated gas: {}", batch_tx.inner.gas.unwrap_or(0))?;

        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut batch_tx, sender).await?;
        } else {
            maybe_print_fee_token(Some(provider.as_ref()), fee_token).await?;
        }

        // Sign and send.
        let tx_hash = match batch_signer {
            BatchSigner::Wallet(wallet) => {
                let provider_with_wallet =
                    alloy_provider::ProviderBuilder::<_, _, TempoNetwork>::default()
                        .wallet(wallet)
                        .connect_provider(provider.as_ref());

                let pending = provider_with_wallet.send_transaction(batch_tx).await?;
                *pending.tx_hash()
            }
            BatchSigner::TempoKeychain(wallet) => {
                let raw_tx = batch_tx.sign_with_tempo_wallet(&wallet).await?;

                let pending = provider.send_raw_transaction(&raw_tx).await?;
                *pending.tx_hash()
            }
            BatchSigner::Unlocked => {
                let pending = provider.send_transaction(batch_tx).await?;
                *pending.tx_hash()
            }
        };

        sh_println!("Batch transaction sent: {:#x}", tx_hash)?;

        // Checkpoint: stamp the batch hash on all remaining transactions (so that resume
        // detection finds it regardless of which tx it inspects first), register one entry
        // in sequence.pending for drop/timeout tracking, then save.
        for tx in sequence.transactions.iter_mut().skip(remaining_start) {
            tx.hash = Some(tx_hash);
        }
        if !sequence.pending.contains(&tx_hash) {
            sequence.pending.push(tx_hash);
        }
        self.sequence.save(true, false)?;

        // Wait for receipt
        let timeout = self.script_config.config.transaction_timeout;
        let receipt = tokio::time::timeout(
            Duration::from_secs(timeout),
            wait_for_batch_receipt(provider.as_ref(), tx_hash, self.args.confirmations),
        )
        .await
        .map_err(|_| eyre::eyre!("Timeout waiting for batch transaction receipt (tx: {tx_hash:#x}). Run with --resume to retry."))??
        .ok_or_else(|| eyre::eyre!("Batch transaction {tx_hash:#x} was dropped from the mempool. Run with --resume to retry."))?;

        let success = receipt.status();
        if success {
            sh_println!(
                "Batch transaction confirmed in block {}",
                receipt.block_number.unwrap_or(0)
            )?;
        } else {
            bail!("Batch transaction failed (reverted)");
        }

        let sequence = self.sequence.sequences_mut().get_mut(0).unwrap();
        sequence.remove_pending(tx_hash);

        // Receipts are pushed 1:1 with the remaining (not-yet-receipted) transactions.
        let remaining_len = sequence.transactions.len() - remaining_start;
        if calls.len() != remaining_len {
            bail!(
                "batch call count ({}) does not match remaining transactions ({}); \
                 refusing to push misaligned receipts",
                calls.len(),
                remaining_len
            );
        }
        // Only carry through contract_address for actual deployments; plain calls also
        // store the callee in `contract_address`, which would otherwise be copied into
        // the receipt and treated as a fresh deployment by downstream consumers
        // (broadcast JSON, verifier).
        let per_tx_addresses: Vec<Option<Address>> = sequence
            .transactions
            .iter()
            .skip(remaining_start)
            .map(|tx| match tx.call_kind {
                CallKind::Create | CallKind::Create2 => tx.contract_address,
                _ => None,
            })
            .collect();

        for (idx, addr) in per_tx_addresses.iter().enumerate() {
            if let Some(addr) = addr {
                sh_println!("  call[{idx}] deployed at: {addr:#x}")?;
            }
        }

        // gasUsed reflects the whole batch; per-call attribution is unavailable from the receipt.
        for addr in &per_tx_addresses {
            let mut tx_receipt = receipt.clone();
            tx_receipt.contract_address = *addr;
            sequence.receipts.push(tx_receipt);
        }

        let chain = sequence.chain;
        let _ = sequence;

        self.sequence.save(true, false)?;

        let total_gas = receipt.gas_used();
        let gas_price = receipt.effective_gas_price() as u64;
        let total_paid = total_gas * gas_price;
        let paid = format_units(total_paid, 18).unwrap_or_else(|_| "N/A".to_string());
        let gas_price_gwei = format_units(gas_price, 9).unwrap_or_else(|_| "N/A".to_string());

        let token_symbol = NamedChain::try_from(chain)
            .unwrap_or_default()
            .native_currency_symbol()
            .unwrap_or("ETH");
        sh_println!(
            "\nTotal Paid: {} {} ({} gas * {} gwei)",
            paid.trim_end_matches('0'),
            token_symbol,
            total_gas,
            gas_price_gwei.trim_end_matches('0').trim_end_matches('.')
        )?;

        if !shell::is_json() {
            sh_println!("\n\n==========================")?;
            sh_println!("\nBATCH EXECUTION COMPLETE & SUCCESSFUL.")?;
            sh_println!("All {} calls executed atomically in a single transaction.", calls.len())?;
        }

        Ok(BroadcastedState {
            args: self.args,
            script_config: self.script_config,
            build_data: self.build_data,
            sequence: self.sequence,
        })
    }
}

async fn wait_for_batch_receipt<N: Network>(
    provider: &RootProvider<N>,
    tx_hash: TxHash,
    confirmations: u64,
) -> Result<Option<N::ReceiptResponse>> {
    loop {
        if let Some(receipt) = provider.get_transaction_receipt(tx_hash).await?
            && let Some(receipt_block) = receipt.block_number()
        {
            let latest_block = provider.get_block_number().await?;
            if latest_block >= receipt_block.saturating_add(confirmations.saturating_sub(1)) {
                return Ok(Some(receipt));
            }
        }

        if provider.get_transaction_by_hash(tx_hash).await?.is_none() {
            return Ok(None);
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn estimate_gas<N: Network, P: Provider<N>>(
    tx: &mut N::TransactionRequest,
    provider: &P,
    estimate_multiplier: u64,
    tempo_browser: bool,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    // if already set, some RPC endpoints might simply return the gas value that is already
    // set in the request and omit the estimate altogether, so we remove it here
    tx.reset_gas_limit();

    let request =
        if tempo_browser { tx.browser_wallet_gas_estimation_request() } else { tx.clone() };
    tx.set_gas_limit(
        provider.estimate_gas(request).await.wrap_err("Failed to estimate gas for tx")?
            * estimate_multiplier
            / 100,
    );
    Ok(())
}

/// Returns `caller`'s nonce at an already resolved fork block.
pub(super) async fn next_nonce_resolved(
    caller: Address,
    evm_opts: &EvmOpts,
    fork: &ResolvedFork,
) -> eyre::Result<u64> {
    evm_opts.transaction_count_at_resolved_fork(caller, fork).await
}

fn reject_access_key_create<N: Network>(
    tx: &N::TransactionRequest,
    uses_access_key: bool,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if uses_access_key && tx.tempo_calls().iter().any(|(to, _)| to.is_create()) {
        bail!("Tempo access-key transactions cannot use CREATE");
    }
    Ok(())
}

fn convert_tempo_aa_create<N: Network>(tx: &mut N::TransactionRequest)
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if tx.is_tempo_aa() {
        tx.convert_create_to_call();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{
        Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom, TxEnvelope,
        transaction::SignerRecoverable,
    };
    use alloy_eips::BlockId;
    use alloy_network::Ethereum;
    use alloy_primitives::{Bloom, address, hex};
    use alloy_rpc_types::TransactionReceipt;
    use alloy_signer::Signer;
    use forge_script_sequence::TransactionWithMetadata;

    const ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ACCESS_KEY_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";
    const SIGNED_TX: &[u8] = &hex!(
        "02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000016480c001a070d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0eca052eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a"
    );
    const OTHER_SIGNED_TX: &[u8] = &hex!(
        "02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c001a0cce9a61187b5d18a89ecd27ec675e3b3f10d37f165627ef89a15a7fe76395ce8a07537f5bffb358ffbef22cda84b1c92f7211723f9e09ae037e81686805d3e5505"
    );

    #[tokio::test(flavor = "multi_thread")]
    async fn next_nonce_uses_exact_fork_hash() {
        let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
        let provider = handle.http_provider();
        let sender = handle.dev_accounts().next().unwrap();
        let recipient = Address::with_last_byte(1);

        let receipt = provider
            .send_transaction(
                TransactionRequest::default()
                    .from(sender)
                    .to(recipient)
                    .value(U256::from(1))
                    .into(),
            )
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        let block_number = receipt.block_number.unwrap();
        let evm_opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            fork_block_number: Some(block_number),
            ..Default::default()
        };
        let fork = evm_opts.resolve_fork().await.unwrap().unwrap();
        assert_eq!(next_nonce_resolved(sender, &evm_opts, &fork).await.unwrap(), 1);

        provider
            .raw_request::<_, ()>("anvil_reorg".into(), (1_u64, Vec::<serde_json::Value>::new()))
            .await
            .unwrap();
        assert_eq!(
            provider
                .get_transaction_count(sender)
                .block_id(BlockId::number(block_number))
                .await
                .unwrap(),
            0
        );

        match next_nonce_resolved(sender, &evm_opts, &fork).await {
            Ok(0) => panic!("the exact lookup fell back to the replacement block"),
            Ok(1) | Err(_) => {}
            Ok(nonce) => panic!("unexpected nonce: {nonce}"),
        }
    }

    #[test]
    fn access_key_signer_takes_precedence_over_same_sender_wallet() {
        let root = foundry_wallets::utils::create_private_key_signer(ROOT_PRIVATE_KEY).unwrap();
        let root_address = root.address();
        let access_key =
            foundry_wallets::utils::create_local_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let access_key_address = access_key.address();
        let mut eth_wallets = AddressHashMap::default();
        eth_wallets.insert(root_address, EthereumWallet::new(root));
        let mut access_keys = HashMap::default();
        access_keys.insert(
            SignerScope::new(4217, root_address),
            TempoAccountsWallet::from_secp256k1(root_address, access_key, None).with_chain_id(4217),
        );
        let send_kind =
            SendTransactionsKind::<Ethereum>::Raw { eth_wallets, browser: None, access_keys };

        let tx = TransactionRequest { from: Some(root_address), ..Default::default() };
        let sender = send_kind.for_sender(4217, &root_address, tx).unwrap();

        match sender {
            SendTransactionKind::AccessKey(_, wallet) => {
                assert_eq!(wallet.key_id().unwrap(), access_key_address);
                assert_eq!(wallet.account(), root_address);
            }
            _ => panic!("expected access key signer"),
        }
    }

    #[test]
    fn access_key_signer_is_scoped_to_chain() {
        let root = foundry_wallets::utils::create_private_key_signer(ROOT_PRIVATE_KEY).unwrap();
        let root_address = root.address();
        let access_key =
            foundry_wallets::utils::create_local_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let mut eth_wallets = AddressHashMap::default();
        eth_wallets.insert(root_address, EthereumWallet::new(root));
        let mut access_keys = HashMap::default();
        access_keys.insert(
            SignerScope::new(4217, root_address),
            TempoAccountsWallet::from_secp256k1(root_address, access_key, None).with_chain_id(4217),
        );
        let send_kind =
            SendTransactionsKind::<Ethereum>::Raw { eth_wallets, browser: None, access_keys };

        let tx = TransactionRequest { from: Some(root_address), ..Default::default() };
        let sender = send_kind.for_sender(1, &root_address, tx).unwrap();

        match sender {
            SendTransactionKind::Raw(_, wallet) => {
                assert_eq!(wallet.default_signer().address(), root_address);
            }
            _ => panic!("expected root wallet signer for non-session chain"),
        }
    }

    #[test]
    fn remaining_unsigned_transactions_skip_completed_transactions() {
        let completed = address!("0x1111111111111111111111111111111111111111");
        let remaining_sender = address!("0x2222222222222222222222222222222222222222");
        let mut sequence = ScriptSequence::<Ethereum> {
            chain: 4217,
            transactions: [script_tx(completed), script_tx(remaining_sender)].into(),
            receipts: vec![receipt()],
            ..Default::default()
        };

        let remaining =
            remaining_unsigned_transactions(std::slice::from_ref(&sequence)).collect::<Vec<_>>();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].from, remaining_sender);
        assert_eq!(remaining[0].chain, 4217);

        sequence.receipts.push(receipt());
        let remaining =
            remaining_unsigned_transactions(std::slice::from_ref(&sequence)).collect::<Vec<_>>();
        assert!(remaining.is_empty());

        let completed_sequence = ScriptSequence::<Ethereum> {
            chain: 1,
            transactions: [script_tx(completed)].into(),
            receipts: vec![receipt()],
            ..Default::default()
        };
        let remaining_sequence = ScriptSequence::<Ethereum> {
            chain: 4217,
            transactions: [script_tx(remaining_sender)].into(),
            ..Default::default()
        };

        let remaining = remaining_unsigned_transactions(&[completed_sequence, remaining_sequence])
            .collect::<Vec<_>>();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].chain, 4217);
    }

    #[test]
    fn recovered_signed_payload_does_not_require_a_signer() {
        let dir = tempfile::tempdir().unwrap();
        let mut sequence = ScriptSequence::<Ethereum> {
            chain: 1,
            transactions: [planned_tx(SIGNED_TX)].into(),
            ..Default::default()
        };
        sequence.paths = Some((dir.path().join("broadcast.json"), dir.path().join("cache.json")));
        let mut sequence = ScriptSequenceKind::new_single(sequence, false).unwrap();
        let payload = Bytes::from_static(SIGNED_TX);
        sequence.persist_signed_payload(0, 0, payload).unwrap();

        assert!(remaining_unsigned_transactions_for_recovery(&sequence).is_empty());
    }

    #[test]
    fn recovered_sender_still_requires_sequential_ordering() {
        let dir = tempfile::tempdir().unwrap();
        let unsigned = address!("0x2222222222222222222222222222222222222222");
        let mut sequence = ScriptSequence::<Ethereum> {
            chain: 1,
            transactions: [planned_tx(SIGNED_TX), script_tx(unsigned)].into(),
            ..Default::default()
        };
        sequence.paths = Some((dir.path().join("broadcast.json"), dir.path().join("cache.json")));
        let mut sequence = ScriptSequenceKind::new_single(sequence, false).unwrap();
        sequence.persist_signed_payload(0, 0, Bytes::from_static(SIGNED_TX)).unwrap();

        let required = remaining_unsigned_transactions_for_recovery(&sequence);
        assert_eq!(required.iter().map(|tx| tx.from).collect::<Vec<_>>(), [unsigned]);
        assert_eq!(remaining_sender_addresses(&sequence).len(), 2);
    }

    #[test]
    fn signed_only_sequences_remain_sequential() {
        assert!(should_broadcast_sequentially(false, false, 0, 1, true));
        assert!(!should_broadcast_sequentially(false, false, 1, 1, true));
    }

    #[test]
    fn recovered_completion_is_matched_by_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut deployment = ScriptSequence::<Ethereum> {
            chain: 1,
            transactions: [planned_tx(SIGNED_TX), planned_tx(OTHER_SIGNED_TX)].into(),
            ..Default::default()
        };
        deployment.paths = Some((dir.path().join("broadcast.json"), dir.path().join("cache.json")));
        let mut sequence = ScriptSequenceKind::new_single(deployment, false).unwrap();
        sequence.persist_signed_payload(0, 0, Bytes::from_static(SIGNED_TX)).unwrap();
        let second_hash =
            sequence.persist_signed_payload(0, 1, Bytes::from_static(OTHER_SIGNED_TX)).unwrap();
        let mut second_receipt = receipt();
        second_receipt.transaction_hash = second_hash;
        sequence.sequences_mut()[0].receipts.push(second_receipt);

        assert_eq!(remaining_operation_indices(&sequence, 0), [0]);
    }

    #[test]
    fn externally_signed_completion_uses_the_persisted_hash() {
        let dir = tempfile::tempdir().unwrap();
        let sender = address!("0x1111111111111111111111111111111111111111");
        let mut deployment = ScriptSequence::<Ethereum> {
            chain: 1,
            transactions: [script_tx(sender), script_tx(sender)].into(),
            receipts: vec![receipt()],
            ..Default::default()
        };
        deployment.transactions[1].hash = Some(deployment.receipts[0].transaction_hash());
        deployment.paths = Some((dir.path().join("broadcast.json"), dir.path().join("cache.json")));
        let sequence = ScriptSequenceKind::new_single(deployment, false).unwrap();

        assert_eq!(remaining_operation_indices(&sequence, 0), [0]);
    }

    #[test]
    fn duplicate_receipts_do_not_complete_an_unsigned_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut deployment = ScriptSequence::<Ethereum> {
            chain: 1,
            transactions: [planned_tx(SIGNED_TX), planned_tx(OTHER_SIGNED_TX)].into(),
            ..Default::default()
        };
        deployment.paths = Some((dir.path().join("broadcast.json"), dir.path().join("cache.json")));
        let mut sequence = ScriptSequenceKind::new_single(deployment, false).unwrap();
        let first_hash =
            sequence.persist_signed_payload(0, 0, Bytes::from_static(SIGNED_TX)).unwrap();
        let mut first_receipt = receipt();
        first_receipt.transaction_hash = first_hash;
        sequence.sequences_mut()[0].receipts = vec![first_receipt.clone(), first_receipt];

        assert_eq!(remaining_operation_indices(&sequence, 0), [1]);
    }

    #[test]
    fn remaining_transactions_skip_receipt_prefix() {
        let completed = address!("0x1111111111111111111111111111111111111111");
        let second = address!("0x2222222222222222222222222222222222222222");
        let third = address!("0x3333333333333333333333333333333333333333");
        let mut sequence = ScriptSequence::<Ethereum> {
            chain: 4217,
            transactions: [script_tx(completed), script_tx(second), script_tx(third)].into(),
            receipts: vec![receipt()],
            ..Default::default()
        };

        let remaining =
            remaining_transactions(&sequence).map(|tx| tx.from().unwrap()).collect::<Vec<_>>();

        assert_eq!(remaining, vec![second, third]);

        sequence.receipts = (0..4).map(|_| receipt()).collect();
        assert!(remaining_transactions(&sequence).next().is_none());
    }

    #[tokio::test]
    async fn access_key_sets_key_id_before_estimation() {
        let root_address = address!("0x1111111111111111111111111111111111111111");
        let access_key =
            foundry_wallets::utils::create_local_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let access_key_address = access_key.address();
        let access_key_wallet =
            TempoAccountsWallet::from_secp256k1(root_address, access_key, None).with_chain_id(4217);
        let mut sender = SendTransactionKind::<TempoNetwork>::AccessKey(
            TempoTransactionRequest {
                inner: TransactionRequest { from: Some(root_address), ..Default::default() },
                ..Default::default()
            },
            Box::new(access_key_wallet),
        );
        let provider =
            RootProvider::<TempoNetwork>::new_http("http://localhost:8545".parse().unwrap());

        sender
            .prepare(
                &provider,
                false,
                true,
                false,
                100,
                None,
                Some(Chain::from_named(NamedChain::Mainnet)),
            )
            .await
            .unwrap();

        match sender {
            SendTransactionKind::AccessKey(tx, _) => {
                assert_eq!(tx.key_id, Some(access_key_address));
            }
            _ => panic!("expected access key transaction"),
        }
    }

    #[test]
    fn tempo_aa_create_moves_deployment_into_calls() {
        let mut tx = TempoTransactionRequest {
            inner: TransactionRequest { to: Some(TxKind::Create), ..Default::default() },
            fee_token: Some(address!("0x20c0000000000000000000000000000000000000")),
            ..Default::default()
        };

        convert_tempo_aa_create::<TempoNetwork>(&mut tx);

        assert!(tx.inner.to.is_none());
        assert_eq!(tx.calls.len(), 1);
        assert!(tx.calls[0].to.is_create());
    }

    #[test]
    fn tempo_access_key_create_is_rejected_before_preparation() {
        let tx = TempoTransactionRequest {
            inner: TransactionRequest { to: Some(TxKind::Create), ..Default::default() },
            ..Default::default()
        };

        let error = reject_access_key_create::<TempoNetwork>(&tx, true).unwrap_err();

        assert!(error.to_string().contains("Tempo access-key transactions cannot use CREATE"));
    }

    fn script_tx(from: Address) -> TransactionWithMetadata<Ethereum> {
        TransactionWithMetadata::from_tx_request(TransactionMaybeSigned::new(TransactionRequest {
            from: Some(from),
            ..Default::default()
        }))
    }

    fn planned_tx(payload: &[u8]) -> TransactionWithMetadata<Ethereum> {
        let envelope = TxEnvelope::decode_2718_exact(payload).unwrap();
        let from = envelope.recover_signer().unwrap();
        let mut request: TransactionRequest = envelope.into();
        request.from = Some(from);
        TransactionWithMetadata::from_tx_request(TransactionMaybeSigned::new(request))
    }

    fn receipt() -> TransactionReceipt {
        TransactionReceipt {
            inner: ReceiptEnvelope::Legacy(ReceiptWithBloom {
                receipt: Receipt {
                    status: Eip658Value::success(),
                    cumulative_gas_used: 0,
                    logs: vec![],
                },
                logs_bloom: Bloom::ZERO,
            }),
            transaction_hash: Default::default(),
            transaction_index: None,
            block_hash: None,
            block_number: None,
            gas_used: 0,
            effective_gas_price: 0,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Address::ZERO,
            to: None,
            contract_address: None,
        }
    }
}
