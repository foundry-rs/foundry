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
use alloy_eips::{BlockId, eip2718::Encodable2718};
use alloy_network::{
    EthereumWallet, Network, NetworkTransactionBuilder, ReceiptResponse, TransactionBuilder,
};
use alloy_primitives::{
    Address, TxHash, TxKind, U256, keccak256,
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
    provider::{ProviderBuilder, try_get_http_provider},
    shell,
    tempo::{
        KeyEntry, KeysFile, TempoSponsor, WALLET_KEYS_PATH, decode_key_authorization,
        maybe_print_fee_token, tempo_home,
    },
};
use foundry_config::Config;
use foundry_evm::core::{
    constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH,
    evm::{FoundryEvmNetwork, TempoEvmNetwork},
};
use foundry_wallets::{
    TempoAccessKeyConfig, WalletSigner, tempo::TempoLookup, wallet_browser::signer::BrowserSigner,
};
use futures::{FutureExt, StreamExt, future::join_all, stream::FuturesUnordered};
use itertools::Itertools;
use revm_inspectors::tracing::types::CallKind;
use tempo_alloy::{TempoNetwork, rpc::TempoTransactionRequest};
use tempo_primitives::transaction::Call;

pub async fn estimate_gas<N: Network, P: Provider<N>>(
    tx: &mut N::TransactionRequest,
    provider: &P,
    estimate_multiplier: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    // if already set, some RPC endpoints might simply return the gas value that is already
    // set in the request and omit the estimate altogether, so we remove it here
    tx.reset_gas_limit();

    tx.set_gas_limit(
        provider.estimate_gas(tx.clone()).await.wrap_err("Failed to estimate gas for tx")?
            * estimate_multiplier
            / 100,
    );
    Ok(())
}

pub async fn next_nonce(
    caller: Address,
    provider_url: &str,
    block_number: Option<u64>,
) -> eyre::Result<u64> {
    let provider = try_get_http_provider(provider_url)
        .wrap_err_with(|| format!("bad fork_url provider: {provider_url}"))?;

    let block_id = block_number.map_or(BlockId::latest(), BlockId::number);
    Ok(provider.get_transaction_count(caller).block_id(block_id).await?)
}

/// Represents how to send a single transaction.
#[derive(Clone)]
pub enum SendTransactionKind<'a, N: Network> {
    Unlocked(N::TransactionRequest),
    Raw(N::TransactionRequest, &'a EthereumWallet),
    Browser(N::TransactionRequest, &'a BrowserSigner<N>),
    Signed(N::TxEnvelope),
    AccessKey(N::TransactionRequest, &'a WalletSigner, &'a TempoAccessKeyConfig),
}

impl<'a, N: Network> SendTransactionKind<'a, N>
where
    N::TxEnvelope: From<Signed<N::UnsignedTx>>,
    N::UnsignedTx: SignableTransaction<Signature>,
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    /// Prepares the transaction for broadcasting by synchronizing nonce and estimating gas.
    ///
    /// This method performs two key operations:
    /// 1. Nonce synchronization: Waits for the provider's nonce to catch up to the expected
    ///    transaction nonce when doing sequential broadcast
    /// 2. Gas estimation: Re-estimates gas right before broadcasting for chains that require it
    pub async fn prepare(
        &mut self,
        provider: &RootProvider<N>,
        sequential_broadcast: bool,
        is_fixed_gas_limit: bool,
        estimate_via_rpc: bool,
        estimate_multiplier: u64,
        tempo_sponsor: Option<&TempoSponsor>,
    ) -> Result<()> {
        let (tx, access_key_authorization) = match self {
            Self::Raw(tx, _) | Self::Unlocked(tx) | Self::Browser(tx, _) => (tx, None),
            Self::AccessKey(tx, _, access_key) => {
                tx.set_key_id(access_key.key_address);
                (
                    tx,
                    Some((
                        access_key.wallet_address,
                        access_key.key_address,
                        access_key.key_authorization.as_ref(),
                    )),
                )
            }
            Self::Signed(_) => return Ok(()),
        };

        if sequential_broadcast {
            let from = tx.from().expect("no sender");

            let tx_nonce = tx.nonce().expect("no nonce");
            for attempt in 0..5 {
                let nonce = provider.get_transaction_count(from).await?;
                match nonce.cmp(&tx_nonce) {
                    Ordering::Greater => {
                        bail!(
                            "EOA nonce changed unexpectedly while sending transactions. Expected {tx_nonce} got {nonce} from provider."
                        )
                    }
                    Ordering::Less => {
                        if attempt == 4 {
                            bail!(
                                "After 5 attempts, provider nonce ({nonce}) is still behind expected nonce ({tx_nonce})."
                            )
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

        if let Some((wallet_address, key_address, key_authorization)) = access_key_authorization {
            tx.prepare_access_key_authorization(
                provider,
                wallet_address,
                key_address,
                key_authorization,
            )
            .await?;
        }

        // Chains which use `eth_estimateGas` are being sent sequentially and require their
        // gas to be re-estimated right before broadcasting.
        if !is_fixed_gas_limit && estimate_via_rpc {
            estimate_gas(tx, provider, estimate_multiplier).await?;
        }

        if let Some(sponsor) = tempo_sponsor {
            let from = tx.from().expect("no sender");
            sponsor.attach_and_print::<N>(tx, from).await?;
        }
        maybe_print_fee_token(Some(provider), tx.fee_token()).await?;

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
            Self::AccessKey(tx, signer, access_key) => {
                debug!("sending transaction via tempo access key: {:?}", tx);

                let raw_tx = tx
                    .sign_with_access_key(
                        provider.as_ref(),
                        signer,
                        access_key.wallet_address,
                        access_key.key_address,
                        access_key.key_authorization.as_ref(),
                    )
                    .await?;

                let pending = provider.send_raw_transaction(&raw_tx).await?;
                Ok(*pending.tx_hash())
            }
        }
    }

    /// Prepares and sends the transaction in one operation.
    ///
    /// This is a convenience method that combines [`prepare`](Self::prepare) and
    /// [`send`](Self::send) into a single call.
    pub async fn prepare_and_send(
        mut self,
        provider: Arc<RootProvider<N>>,
        sequential_broadcast: bool,
        is_fixed_gas_limit: bool,
        estimate_via_rpc: bool,
        estimate_multiplier: u64,
        tempo_sponsor: Option<&TempoSponsor>,
    ) -> Result<TxHash> {
        self.prepare(
            &provider,
            sequential_broadcast,
            is_fixed_gas_limit,
            estimate_via_rpc,
            estimate_multiplier,
            tempo_sponsor,
        )
        .await?;

        self.send(provider).await
    }
}

fn build_lookup(entry: &KeyEntry) -> Result<TempoLookup> {
    let Some(ref key) = entry.key else {
        return Ok(TempoLookup::NotFound);
    };
    let signer = foundry_wallets::utils::create_private_key_signer(key)?;
    let Some(key_address) = entry.key_address.filter(|ka| *ka != entry.wallet_address) else {
        return Ok(TempoLookup::Direct(signer));
    };
    let key_authorization =
        entry.key_authorization.as_deref().map(decode_key_authorization).transpose()?;
    let config = TempoAccessKeyConfig {
        wallet_address: entry.wallet_address,
        key_address,
        key_authorization,
    };
    Ok(TempoLookup::Keychain(signer, Box::new(config)))
}

/// Like [`build_lookup`] but strips `key_authorization` since the entry is chain-0 and its
/// authorization was not issued for the target chain.
fn build_lookup_chain0_fallback(entry: &KeyEntry, chain: u64) -> Result<TempoLookup> {
    let Some(ref key) = entry.key else {
        return Ok(TempoLookup::NotFound);
    };
    let signer = foundry_wallets::utils::create_private_key_signer(key)?;
    let Some(key_address) = entry.key_address.filter(|ka| *ka != entry.wallet_address) else {
        return Ok(TempoLookup::Direct(signer));
    };
    if entry.key_authorization.is_some() {
        warn!(
            "keys.toml entry for {} has no chain_id — \
             key_authorization ignored for chain {chain} broadcast",
            entry.wallet_address
        );
    }
    let config = TempoAccessKeyConfig {
        wallet_address: entry.wallet_address,
        key_address,
        key_authorization: None,
    };
    Ok(TempoLookup::Keychain(signer, Box::new(config)))
}

/// Looks up a Tempo wallet signer scoped to the transaction chain.
///
/// Prefers an entry whose `(wallet_address, chain_id)` both match. Falls back to an entry with
/// `chain_id == 0` (the value when the field is absent) so that `keys.toml` files written by older
/// Tempo clients (which omit `chain_id`) continue to work.
pub(crate) fn lookup_signer_for_chain(from: Address, chain: u64) -> Result<TempoLookup> {
    let Some(path) = tempo_home().map(|home| home.join(WALLET_KEYS_PATH)) else {
        return Ok(TempoLookup::NotFound);
    };
    if !path.is_file() {
        return Ok(TempoLookup::NotFound);
    }

    let contents = std::fs::read_to_string(&path)?;
    let file: KeysFile = toml::from_str(&contents)?;

    lookup_signer_in(from, chain, &file)
}

fn lookup_signer_in(from: Address, chain: u64, file: &KeysFile) -> Result<TempoLookup> {
    let mut fallback: Option<&KeyEntry> = None;
    for entry in &file.keys {
        if entry.wallet_address != from {
            continue;
        }
        if entry.chain_id == chain {
            if entry.key.is_some() {
                return build_lookup(entry);
            }
            // exact chain match but no key -> keep searching for a fallback
            continue;
        }
        if entry.chain_id == 0 && fallback.is_none() {
            fallback = Some(entry);
        }
    }
    fallback.map(|e| build_lookup_chain0_fallback(e, chain)).unwrap_or(Ok(TempoLookup::NotFound))
}

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
        access_keys: HashMap<SignerScope, (WalletSigner, TempoAccessKeyConfig)>,
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
                    bail!("Sender address {:?} is not unlocked", addr)
                }
                Ok(SendTransactionKind::Unlocked(tx))
            }
            Self::Raw { eth_wallets, browser, access_keys } => {
                if let Some((signer, config)) = access_keys.get(&SignerScope::new(chain, *addr)) {
                    Ok(SendTransactionKind::AccessKey(tx, signer, config))
                } else if let Some(wallet) = eth_wallets.get(addr) {
                    Ok(SendTransactionKind::Raw(tx, wallet))
                } else if let Some(b) = browser
                    && b.address() == *addr
                {
                    Ok(SendTransactionKind::Browser(tx, b))
                } else {
                    bail!("No matching signer for {:?} found", addr)
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
        let futs = self
            .sequence
            .sequences_mut()
            .iter_mut()
            .enumerate()
            .map(|(sequence_idx, sequence)| async move {
                let rpc_url = sequence.rpc_url();
                let provider = Arc::new(ProviderBuilder::new(rpc_url).build()?);
                progress_ref
                    .wait_for_pending(
                        sequence_idx,
                        sequence,
                        &provider,
                        self.script_config.config.transaction_timeout,
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
    pub async fn broadcast(mut self) -> Result<BroadcastedState<FEN>> {
        let remaining_transactions =
            remaining_unsigned_transactions(self.sequence.sequences()).collect::<Vec<_>>();
        let required_addresses =
            remaining_transactions.iter().map(|tx| tx.from).collect::<AddressHashSet>();

        if required_addresses.contains(&Config::DEFAULT_SENDER) {
            eyre::bail!(
                "You seem to be using Foundry's default sender. Be sure to set your own --sender."
            );
        }

        let send_kind = if self.args.unlocked {
            SendTransactionsKind::Unlocked(required_addresses.clone())
        } else {
            let expected_session_sender = script_session_expected_sender_if_configured(
                &self.script_config.tempo,
                &required_addresses,
            )?;

            // For addresses without an explicit signer, try Tempo keys.toml fallback.
            let mut access_keys: HashMap<SignerScope, (WalletSigner, TempoAccessKeyConfig)> =
                HashMap::default();
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

            let mut direct_signers: AddressHashMap<WalletSigner> = AddressHashMap::default();
            let mut missing_addresses = Vec::new();

            for tx in &remaining_transactions {
                let scope = tx.scope();
                if !signers.contains(&tx.from) && !access_keys.contains_key(&scope) {
                    match lookup_signer_for_chain(tx.from, tx.chain) {
                        Ok(TempoLookup::Direct(signer)) => {
                            direct_signers.insert(tx.from, signer);
                        }
                        Ok(TempoLookup::Keychain(signer, config)) => {
                            access_keys.insert(scope, (signer, *config));
                        }
                        _ => {
                            missing_addresses.push(tx.from);
                        }
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
            let mut eth_wallets: AddressHashMap<EthereumWallet> =
                signers.into_iter().map(|(addr, signer)| (addr, signer.into())).collect();
            for (addr, signer) in direct_signers {
                eth_wallets.insert(addr, signer.into());
            }

            SendTransactionsKind::Raw { eth_wallets, browser: self.browser_wallet, access_keys }
        };

        let tempo_sponsor = self.script_config.tempo.sponsor_config().await?.map(Arc::new);
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
            let mut sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

            let provider = Arc::new(ProviderBuilder::new(sequence.rpc_url()).build()?);
            let already_broadcasted = sequence.receipts.len();

            let seq_progress = progress.get_sequence_progress(i, sequence);

            if already_broadcasted < sequence.transactions.len() {
                let is_legacy = Chain::from(sequence.chain).is_legacy() || self.args.legacy;
                // Make a one-time gas price estimation
                let (gas_price, eip1559_fees) = match (
                    is_legacy,
                    self.args.with_gas_price,
                    self.args.priority_gas_price,
                ) {
                    (true, Some(gas_price), _) => (Some(gas_price.to()), None),
                    (true, None, _) => (Some(provider.get_gas_price().await?), None),
                    (false, Some(max_fee_per_gas), Some(max_priority_fee_per_gas)) => {
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
                    (false, _, _) => {
                        let mut fees = provider.estimate_eip1559_fees().await.wrap_err("Failed to estimate EIP1559 fees. This chain might not support EIP1559, try adding --legacy to your command.")?;

                        // When using --browser, the browser wallet may override the
                        // priority fee with its own estimate (from
                        // eth_maxPriorityFeePerGas) without adjusting maxFeePerGas,
                        // leading to maxPriorityFeePerGas > maxFeePerGas.
                        // This is common on OP Stack chains (e.g. Base) where
                        // eth_feeHistory returns empty reward arrays, causing the
                        // estimator to fall back to a 1 wei priority fee.
                        if matches!(&send_kind, SendTransactionsKind::Raw { browser: Some(_), .. })
                            && let Ok(suggested_tip) = provider.get_max_priority_fee_per_gas().await
                            && suggested_tip > fees.max_priority_fee_per_gas
                        {
                            fees.max_fee_per_gas += suggested_tip - fees.max_priority_fee_per_gas;
                            fees.max_priority_fee_per_gas = suggested_tip;
                        }

                        if let Some(gas_price) = self.args.with_gas_price {
                            fees.max_fee_per_gas = gas_price.to();
                        }

                        if let Some(priority_gas_price) = self.args.priority_gas_price {
                            fees.max_priority_fee_per_gas = priority_gas_price.to();
                        }

                        (None, Some(fees))
                    }
                };

                // Iterate through transactions, matching the `from` field with the associated
                // wallet. Then send the transaction. Panics if we find a unknown `from`
                let transactions = sequence
                    .transactions
                    .iter()
                    .skip(already_broadcasted)
                    .map(|tx_with_metadata| {
                        let is_fixed_gas_limit = tx_with_metadata.is_fixed_gas_limit;

                        let kind = match tx_with_metadata.tx().clone() {
                            TransactionMaybeSigned::Signed { tx, .. } => {
                                if tempo_sponsor.is_some() {
                                    eyre::bail!(
                                        "cannot attach Tempo sponsor signature to an already signed script transaction"
                                    );
                                }
                                SendTransactionKind::Signed(tx)
                            }
                            TransactionMaybeSigned::Unsigned(mut tx) => {
                                let from = tx.from().expect("No sender for onchain transaction!");

                                tx.set_chain_id(sequence.chain);

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

                                send_kind.for_sender(sequence.chain, &from, tx)?
                            }
                        };

                        Ok((kind, is_fixed_gas_limit))
                    })
                    .collect::<Result<Vec<_>>>()?;

                let estimate_via_rpc = has_different_gas_calc(sequence.chain)
                    || self.script_config.evm_opts.networks.is_tempo()
                    || self.args.skip_simulation;

                // We only wait for a transaction receipt before sending the next transaction, if
                // there is more than one signer. There would be no way of assuring
                // their order otherwise.
                // Or if the chain does not support batched transactions (eg. Arbitrum).
                // Or if we need to invoke eth_estimateGas before sending transactions.
                let sequential_broadcast = estimate_via_rpc
                    || self.args.slow
                    || required_addresses.len() != 1
                    || !has_batch_support(sequence.chain);

                // We send transactions and wait for receipts in batches of 100, since some networks
                // cannot handle more than that.
                let batch_size = if sequential_broadcast { 1 } else { 100 };
                let mut index = already_broadcasted;

                for (batch_number, batch) in transactions.chunks(batch_size).enumerate() {
                    seq_progress.inner.write().set_status(&format!(
                        "Sending transactions [{} - {}]",
                        batch_number * batch_size,
                        batch_number * batch_size + std::cmp::min(batch_size, batch.len()) - 1
                    ));

                    if !batch.is_empty() {
                        let pending_transactions =
                            batch.iter().map(|(kind, is_fixed_gas_limit)| {
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
                                        )
                                        .await;
                                    (res, kind, *is_fixed_gas_limit, 0, None)
                                }
                                .boxed()
                            });

                        let mut buffer = pending_transactions.collect::<FuturesUnordered<_>>();

                        'send: while let Some((
                            res,
                            kind,
                            is_fixed_gas_limit,
                            attempt,
                            original_res,
                        )) = buffer.next().await
                        {
                            if res.is_err()
                                && self.script_config.tempo.sponsor_sig.is_some()
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
                                        )
                                        .await;
                                    (
                                        r,
                                        kind,
                                        is_fixed_gas_limit,
                                        attempt,
                                        original_res.or(Some(res)),
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
                            sequence.add_pending(index, tx_hash);

                            // Checkpoint save
                            self.sequence.save(true, false)?;
                            sequence = self.sequence.sequences_mut().get_mut(i).unwrap();

                            seq_progress.inner.write().tx_sent(tx_hash);
                            index += 1;
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

        let provider = Arc::new(ProviderBuilder::<TempoNetwork>::new(sequence.rpc_url()).build()?);

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
            let receipt_result = tokio::time::timeout(Duration::from_secs(timeout), async {
                loop {
                    if let Some(receipt) = provider.get_transaction_receipt(tx_hash).await? {
                        return Ok::<_, eyre::Error>(Some(receipt));
                    }
                    // If the tx has left the mempool without a receipt it was dropped.
                    if provider.get_transaction_by_hash(tx_hash).await?.is_none() {
                        return Ok(None);
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            })
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
            TempoKeychain(Box<WalletSigner>, Box<TempoAccessKeyConfig>),
        }

        let batch_signer = if self.args.unlocked {
            BatchSigner::Unlocked
        } else if let Some(session) = self.script_config.tempo.session_signer_for_multi_wallet(
            &self.args.wallets,
            Some(sender),
            chain_id,
        )? {
            BatchSigner::TempoKeychain(Box::new(session.signer), Box::new(session.access_key))
        } else {
            let mut signers = self.script_wallets.into_multi_wallet().into_signers()?;
            if let Some(signer) = signers.remove(&sender) {
                BatchSigner::Wallet(EthereumWallet::new(signer))
            } else {
                // Try Tempo keys.toml fallback
                match lookup_signer_for_chain(sender, chain_id)? {
                    TempoLookup::Direct(signer) => BatchSigner::Wallet(EthereumWallet::new(signer)),
                    TempoLookup::Keychain(signer, config) => {
                        BatchSigner::TempoKeychain(Box::new(signer), config)
                    }
                    TempoLookup::NotFound => {
                        bail!("No wallet found for sender {}", sender);
                    }
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
                None => bail!(
                    "Unexpected raw CREATE in --batch mode at position {} — \
                     this is a bug; CREATEs should have been rewritten by the inspector.",
                    call_index + 1
                ),
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

        // Get gas prices - batch transactions are Tempo-only, always use EIP-1559 style fees
        let fees = provider.estimate_eip1559_fees().await?;
        let max_fee_per_gas =
            self.args.with_gas_price.map(|p| p.to()).unwrap_or(fees.max_fee_per_gas);
        let max_priority_fee_per_gas =
            self.args.priority_gas_price.map(|p| p.to()).unwrap_or(fees.max_priority_fee_per_gas);

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

        if let BatchSigner::TempoKeychain(_, ak) = &batch_signer {
            batch_tx.key_id = Some(ak.key_address);
            batch_tx
                .prepare_access_key_authorization(
                    provider.as_ref(),
                    ak.wallet_address,
                    ak.key_address,
                    ak.key_authorization.as_ref(),
                )
                .await?;
        }

        // Estimate gas for the batch transaction
        estimate_gas(&mut batch_tx, provider.as_ref(), self.args.gas_estimate_multiplier).await?;

        sh_println!("Estimated gas: {}", batch_tx.inner.gas.unwrap_or(0))?;

        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut batch_tx, sender).await?;
        }
        maybe_print_fee_token(Some(provider.as_ref()), batch_tx.fee_token()).await?;

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
            BatchSigner::TempoKeychain(signer, access_key) => {
                let raw_tx = batch_tx
                    .sign_with_access_key(
                        provider.as_ref(),
                        &*signer,
                        access_key.wallet_address,
                        access_key.key_address,
                        access_key.key_authorization.as_ref(),
                    )
                    .await?;

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
        let receipt = tokio::time::timeout(Duration::from_secs(timeout), async {
            loop {
                if let Some(receipt) = provider.get_transaction_receipt(tx_hash).await? {
                    return Ok::<_, eyre::Error>(receipt);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .map_err(|_| eyre::eyre!("Timeout waiting for batch transaction receipt (tx: {tx_hash:#x}). Run with --resume to retry."))??;

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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom};
    use alloy_network::Ethereum;
    use alloy_primitives::{Bloom, address};
    use alloy_rpc_types::TransactionReceipt;
    use alloy_signer::Signer;
    use forge_script_sequence::TransactionWithMetadata;

    const ROOT_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const TEST_ADDR: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    const ACCESS_KEY_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0";

    #[test]
    fn access_key_signer_takes_precedence_over_same_sender_wallet() {
        let root = foundry_wallets::utils::create_private_key_signer(ROOT_PRIVATE_KEY).unwrap();
        let root_address = root.address();
        let access_key =
            foundry_wallets::utils::create_private_key_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let access_key_address = access_key.address();
        let mut eth_wallets = AddressHashMap::default();
        eth_wallets.insert(root_address, EthereumWallet::new(root));
        let mut access_keys = HashMap::default();
        access_keys.insert(
            SignerScope::new(4217, root_address),
            (
                access_key,
                TempoAccessKeyConfig {
                    wallet_address: root_address,
                    key_address: access_key_address,
                    key_authorization: None,
                },
            ),
        );
        let send_kind =
            SendTransactionsKind::<Ethereum>::Raw { eth_wallets, browser: None, access_keys };

        let tx = TransactionRequest { from: Some(root_address), ..Default::default() };
        let sender = send_kind.for_sender(4217, &root_address, tx).unwrap();

        match sender {
            SendTransactionKind::AccessKey(_, signer, access_key) => {
                assert_eq!(signer.address(), access_key_address);
                assert_eq!(access_key.wallet_address, root_address);
            }
            _ => panic!("expected access key signer"),
        }
    }

    #[test]
    fn access_key_signer_is_scoped_to_chain() {
        let root = foundry_wallets::utils::create_private_key_signer(ROOT_PRIVATE_KEY).unwrap();
        let root_address = root.address();
        let access_key =
            foundry_wallets::utils::create_private_key_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let access_key_address = access_key.address();
        let mut eth_wallets = AddressHashMap::default();
        eth_wallets.insert(root_address, EthereumWallet::new(root));
        let mut access_keys = HashMap::default();
        access_keys.insert(
            SignerScope::new(4217, root_address),
            (
                access_key,
                TempoAccessKeyConfig {
                    wallet_address: root_address,
                    key_address: access_key_address,
                    key_authorization: None,
                },
            ),
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
            foundry_wallets::utils::create_private_key_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let access_key_address = access_key.address();
        let access_key_config = TempoAccessKeyConfig {
            wallet_address: root_address,
            key_address: access_key_address,
            key_authorization: None,
        };
        let mut sender = SendTransactionKind::<TempoNetwork>::AccessKey(
            TempoTransactionRequest {
                inner: TransactionRequest { from: Some(root_address), ..Default::default() },
                ..Default::default()
            },
            &access_key,
            &access_key_config,
        );
        let provider =
            RootProvider::<TempoNetwork>::new_http("http://localhost:8545".parse().unwrap());

        sender.prepare(&provider, false, true, false, 100, None).await.unwrap();

        match sender {
            SendTransactionKind::AccessKey(tx, _, _) => {
                assert_eq!(tx.key_id, Some(access_key_address));
            }
            _ => panic!("expected access key transaction"),
        }
    }

    fn script_tx(from: Address) -> TransactionWithMetadata<Ethereum> {
        TransactionWithMetadata::from_tx_request(TransactionMaybeSigned::new(TransactionRequest {
            from: Some(from),
            ..Default::default()
        }))
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

    // lookup_signer_in tests

    fn make_entry(addr: Address, chain_id: u64) -> KeyEntry {
        KeyEntry {
            wallet_address: addr,
            chain_id,
            key: Some(ROOT_PRIVATE_KEY.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn lookup_exact_chain_match() {
        let file = KeysFile { keys: vec![make_entry(TEST_ADDR, 31318)] };
        let result = lookup_signer_in(TEST_ADDR, 31318, &file).unwrap();
        assert!(matches!(result, TempoLookup::Direct(_)));
    }

    #[test]
    fn lookup_chain_zero_fallback() {
        // Entry with chain_id omitted (defaults to 0) should match any chain.
        let file = KeysFile { keys: vec![make_entry(TEST_ADDR, 0)] };
        let result = lookup_signer_in(TEST_ADDR, 31318, &file).unwrap();
        assert!(
            matches!(result, TempoLookup::Direct(_)),
            "chain-0 entry should be used as fallback"
        );
    }

    #[test]
    fn lookup_exact_wins_over_chain_zero_fallback() {
        // chain-0 entry comes first; exact match must still win.
        let file = KeysFile { keys: vec![make_entry(TEST_ADDR, 0), make_entry(TEST_ADDR, 31318)] };
        let result = lookup_signer_in(TEST_ADDR, 31318, &file).unwrap();
        assert!(matches!(result, TempoLookup::Direct(_)));
    }

    #[test]
    fn lookup_mismatched_chain_no_fallback_returns_not_found() {
        let file = KeysFile { keys: vec![make_entry(TEST_ADDR, 1)] };
        let result = lookup_signer_in(TEST_ADDR, 31318, &file).unwrap();
        assert!(matches!(result, TempoLookup::NotFound));
    }
}
