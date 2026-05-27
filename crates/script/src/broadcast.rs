use std::{cmp::Ordering, num::NonZeroU64, sync::Arc, time::Duration};

use crate::{
    ScriptArgs, ScriptConfig, build::LinkedBuildData, progress::ScriptProgress,
    sequence::ScriptSequenceKind, verify::BroadcastedState,
};
use alloy_chains::{Chain, NamedChain};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::{BlockId, eip2718::Encodable2718};
use alloy_network::{
    EthereumWallet, Network, NetworkTransactionBuilder, ReceiptResponse, TransactionBuilder,
};
use alloy_primitives::{
    Address, TxHash, TxKind, U256,
    map::{AddressHashMap, AddressHashSet, HashMap},
    utils::format_units,
};
use alloy_provider::{Provider, RootProvider, utils::Eip1559Estimation};
use alloy_rpc_types::TransactionRequest;
use alloy_signer::Signature;
use eyre::{Context, Result, bail};
use forge_script_sequence::ScriptSequence;
use forge_verify::provider::VerificationProviderType;
use foundry_cheatcodes::Wallets;
use foundry_cli::utils::{has_batch_support, has_different_gas_calc};
use foundry_common::{
    FoundryTransactionBuilder, TransactionMaybeSigned,
    provider::{ProviderBuilder, try_get_http_provider},
    shell,
    tempo::{
        ResolvedSessionSigner, TempoSponsor, WALLET_KEYS_PATH, decode_key_authorization, tempo_home,
    },
};
use foundry_config::Config;
use foundry_evm::core::evm::{FoundryEvmNetwork, TempoEvmNetwork};
use foundry_wallets::{
    TempoAccessKeyConfig, WalletSigner, tempo::TempoLookup, wallet_browser::signer::BrowserSigner,
};
use futures::{FutureExt, StreamExt, future::join_all, stream::FuturesUnordered};
use itertools::Itertools;
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
    fn set_access_key_id_for_estimation(&mut self) {
        if let Self::AccessKey(tx, _, access_key) = self {
            tx.set_key_id(access_key.key_address);
        }
    }

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
        let access_key_authorization = match self {
            Self::AccessKey(_, _, access_key) => Some((
                access_key.wallet_address,
                access_key.key_address,
                access_key.key_authorization.clone(),
            )),
            _ => None,
        };

        self.set_access_key_id_for_estimation();

        if let Self::Raw(tx, _)
        | Self::Unlocked(tx)
        | Self::Browser(tx, _)
        | Self::AccessKey(tx, _, _) = self
        {
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

            if let Some((wallet_address, key_address, key_authorization)) =
                access_key_authorization.as_ref()
            {
                tx.prepare_access_key_authorization(
                    provider,
                    *wallet_address,
                    *key_address,
                    key_authorization.as_ref(),
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

struct ScriptSessionSigner {
    chain: u64,
    root_account: Address,
    signer: WalletSigner,
    access_key: TempoAccessKeyConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SignerScope {
    chain: u64,
    sender: Address,
}

impl SignerScope {
    pub(crate) const fn new(chain: u64, sender: Address) -> Self {
        Self { chain, sender }
    }

    pub(crate) const fn sender(&self) -> Address {
        self.sender
    }
}

impl From<ResolvedSessionSigner> for ScriptSessionSigner {
    fn from(resolved: ResolvedSessionSigner) -> Self {
        Self {
            chain: resolved.session.chain_id,
            root_account: resolved.session.root_account,
            signer: resolved.signer,
            access_key: resolved.access_key,
        }
    }
}

impl ScriptSessionSigner {
    /// Resolves the active Tempo session into the access-key signer used by script broadcast.
    ///
    /// The returned entry is keyed by the root account because script transactions still use the
    /// root account as `from`; the temporary session key only changes how the transaction is
    /// authorized and signed.
    fn resolve_for_chain<FEN: FoundryEvmNetwork>(
        script_config: &ScriptConfig<FEN>,
        wallets: &foundry_wallets::MultiWalletOpts,
        expected_sender: Option<Address>,
        expected_chain_id: u64,
    ) -> Result<Option<Self>> {
        script_config
            .tempo
            .session_signer_for_multi_wallet(wallets, expected_sender, expected_chain_id)
            .map(|session| session.map(Into::into))
    }

    /// Resolves the active Tempo session without imposing a command-level chain.
    ///
    /// The returned signer must be keyed by its own session chain before transaction selection.
    fn resolve_any_chain<FEN: FoundryEvmNetwork>(
        script_config: &ScriptConfig<FEN>,
        wallets: &foundry_wallets::MultiWalletOpts,
        expected_sender: Option<Address>,
    ) -> Result<Option<Self>> {
        script_config
            .tempo
            .session_signer_for_multi_wallet_any_chain(wallets, expected_sender)
            .map(|session| session.map(Into::into))
    }
}

fn insert_session_access_key_for_remaining_transactions(
    access_keys: &mut HashMap<SignerScope, (WalletSigner, TempoAccessKeyConfig)>,
    session_signer: ScriptSessionSigner,
    remaining_transactions: &[RemainingScriptTransaction],
) -> Result<()> {
    if let Some(tx) = remaining_transactions
        .iter()
        .find(|tx| tx.from == session_signer.root_account && tx.chain != session_signer.chain)
    {
        eyre::bail!(
            "Tempo session is for chain {}, but a remaining transaction from session root {} is on chain {}",
            session_signer.chain,
            session_signer.root_account,
            tx.chain
        );
    }

    let scope = SignerScope::new(session_signer.chain, session_signer.root_account);
    if remaining_transactions.iter().any(|tx| SignerScope::new(tx.chain, tx.from) == scope) {
        access_keys.insert(scope, (session_signer.signer, session_signer.access_key));
    }

    Ok(())
}

/// Looks up a Tempo wallet signer scoped to the transaction chain.
///
/// Tempo `keys.toml` entries are stored per `(wallet_address, chain_id)`, and access-key
/// authorizations are chain-specific. Script broadcast must preserve that scope instead of picking
/// the first key for a wallet address.
pub(crate) fn lookup_signer_for_chain(from: Address, chain: u64) -> Result<TempoLookup> {
    let Some(path) = tempo_home().map(|home| home.join(WALLET_KEYS_PATH)) else {
        return Ok(TempoLookup::NotFound);
    };
    if !path.is_file() {
        return Ok(TempoLookup::NotFound);
    }

    let contents = std::fs::read_to_string(&path)?;
    let file: foundry_common::tempo::KeysFile = toml::from_str(&contents)?;

    for entry in file.keys {
        if entry.wallet_address != from || entry.chain_id != chain {
            continue;
        }

        let Some(key) = entry.key else {
            continue;
        };

        let signer = foundry_wallets::utils::create_private_key_signer(&key)?;
        let is_direct = entry.key_address.is_none() || entry.key_address == Some(from);
        if is_direct {
            return Ok(TempoLookup::Direct(signer));
        }

        let key_authorization =
            entry.key_authorization.as_deref().map(decode_key_authorization).transpose()?;
        let config = TempoAccessKeyConfig {
            wallet_address: entry.wallet_address,
            // SAFETY: `is_direct` was false, so `key_address` is `Some` and != wallet_address.
            key_address: entry.key_address.unwrap(),
            key_authorization,
        };
        return Ok(TempoLookup::Keychain(signer, Box::new(config)));
    }

    Ok(TempoLookup::NotFound)
}

/// Returns the single sender a Tempo session is allowed to cover.
///
/// Session signing is intentionally fail-closed: a single session access key represents one root
/// account, so scripts with multiple pending senders must not silently mix the session key with
/// other wallets.
pub(crate) fn script_session_expected_sender(
    required_addresses: &AddressHashSet,
) -> Result<Option<Address>> {
    required_addresses
        .iter()
        .copied()
        .at_most_one()
        .map_err(|_| eyre::eyre!("Tempo sessions require a single script sender"))
}

pub(crate) fn script_session_expected_sender_if_configured<FEN: FoundryEvmNetwork>(
    script_config: &ScriptConfig<FEN>,
    required_addresses: &AddressHashSet,
) -> Result<Option<Address>> {
    script_config
        .tempo
        .session_id()?
        .map_or(Ok(None), |_| script_session_expected_sender(required_addresses))
}

pub(crate) struct RemainingScriptTransaction {
    pub(crate) chain: u64,
    pub(crate) from: Address,
}

impl RemainingScriptTransaction {
    pub(crate) const fn scope(&self) -> SignerScope {
        SignerScope::new(self.chain, self.from)
    }
}

pub(crate) fn remaining_unsigned_transactions<N: Network>(
    sequences: &[ScriptSequence<N>],
) -> impl Iterator<Item = RemainingScriptTransaction> + '_ {
    sequences.iter().flat_map(|sequence| {
        sequence.transactions().skip(sequence.receipts.len()).filter(|tx| tx.is_unsigned()).map(
            |tx| RemainingScriptTransaction {
                chain: sequence.chain,
                from: tx.from().expect("missing from"),
            },
        )
    })
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
                &self.script_config,
                &required_addresses,
            )?;

            // For addresses without an explicit signer, try Tempo keys.toml fallback.
            let mut access_keys: HashMap<SignerScope, (WalletSigner, TempoAccessKeyConfig)> =
                HashMap::default();
            if let Some(expected_session_sender) = expected_session_sender
                && let Some(session_signer) = ScriptSessionSigner::resolve_any_chain(
                    &self.script_config,
                    &self.args.wallets,
                    Some(expected_session_sender),
                )?
            {
                insert_session_access_key_for_remaining_transactions(
                    &mut access_keys,
                    session_signer,
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
        if tempo_sponsor.is_some() && self.script_config.tempo.sponsor_sig.is_some() {
            if remaining_transactions.len() > 1 {
                eyre::bail!(
                    "--tempo.sponsor-sig can only sponsor one remaining script transaction; use --tempo.sponsor-signer for multi-transaction scripts"
                );
            }
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

    pub fn verify_preflight_check(&self) -> Result<()> {
        for sequence in self.sequence.sequences() {
            if self.args.verifier.verifier == VerificationProviderType::Etherscan
                && self
                    .script_config
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
        let provider = Arc::new(ProviderBuilder::<TempoNetwork>::new(sequence.rpc_url()).build()?);
        let tempo_sponsor = self.script_config.tempo.sponsor_config().await?;

        // Collect sender addresses - batch mode requires single sender
        let senders: AddressHashSet = sequence
            .transactions()
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

        // Get wallet for signing
        enum BatchSigner {
            Unlocked,
            Wallet(EthereumWallet),
            TempoKeychain(Box<WalletSigner>, Box<TempoAccessKeyConfig>),
        }

        let batch_signer = if self.args.unlocked {
            BatchSigner::Unlocked
        } else if let Some(session_signer) = ScriptSessionSigner::resolve_for_chain(
            &self.script_config,
            &self.args.wallets,
            Some(sender),
            chain_id,
        )? {
            BatchSigner::TempoKeychain(
                Box::new(session_signer.signer),
                Box::new(session_signer.access_key),
            )
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

        // Collect all transactions into Call structs
        // Tempo batch transactions support CREATE only as the first call
        let mut calls: Vec<Call> = Vec::new();
        let mut has_create = false;
        for (idx, tx) in sequence.transactions().enumerate() {
            let to = match tx.to() {
                Some(addr) => TxKind::Call(addr),
                None => {
                    if idx > 0 {
                        bail!(
                            "Contract creation must be the first transaction in --batch mode. \
                             Found CREATE at position {}. Reorder your script or deploy separately.",
                            idx + 1
                        );
                    }
                    if has_create {
                        bail!("Only one contract creation is allowed per --batch transaction.");
                    }
                    has_create = true;
                    TxKind::Create
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

        // Sign and send
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
        .map_err(|_| eyre::eyre!("Timeout waiting for batch transaction receipt"))??;

        let success = receipt.status();
        if success {
            sh_println!(
                "Batch transaction confirmed in block {}",
                receipt.block_number.unwrap_or(0)
            )?;
        } else {
            bail!("Batch transaction failed (reverted)");
        }

        // For CREATE transactions, compute the deployed contract address
        let created_address = if has_create {
            let deployed_addr = sender.create(nonce);
            sh_println!("Contract deployed at: {:#x}", deployed_addr)?;
            Some(deployed_addr)
        } else {
            None
        };

        // Add receipt to sequence for each original transaction.
        // In batch mode, all calls share the same receipt. Set contract_address
        // only for index 0 if CREATE, clear for the rest to prevent the verifier
        // from attempting to verify the same address multiple times.
        for idx in 0..calls.len() {
            let mut tx_receipt = receipt.clone();
            if idx == 0 && has_create {
                tx_receipt.contract_address = created_address;
            } else {
                tx_receipt.contract_address = None;
            }
            sequence.receipts.push(tx_receipt);
        }

        // Mark all transactions as pending with the batch tx hash
        for i in 0..sequence.transactions.len() {
            sequence.add_pending(i, tx_hash);
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
    fn session_sender_requires_single_root_account() {
        let one = address!("0x1111111111111111111111111111111111111111");
        let two = address!("0x2222222222222222222222222222222222222222");
        let single_sender = [one].into_iter().collect();
        let multiple_senders = [one, two].into_iter().collect();

        assert_eq!(script_session_expected_sender(&single_sender).unwrap(), Some(one));
        assert!(script_session_expected_sender(&multiple_senders).is_err());
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
    fn session_access_key_rejects_session_root_on_wrong_chain() {
        let root = foundry_wallets::utils::create_private_key_signer(ROOT_PRIVATE_KEY).unwrap();
        let root_address = root.address();
        let access_key =
            foundry_wallets::utils::create_private_key_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let access_key_address = access_key.address();
        let session_signer = ScriptSessionSigner {
            chain: 4217,
            root_account: root_address,
            signer: access_key,
            access_key: TempoAccessKeyConfig {
                wallet_address: root_address,
                key_address: access_key_address,
                key_authorization: None,
            },
        };
        let remaining = [RemainingScriptTransaction { chain: 1, from: root_address }];
        let mut access_keys = HashMap::default();

        let err = insert_session_access_key_for_remaining_transactions(
            &mut access_keys,
            session_signer,
            &remaining,
        )
        .unwrap_err();

        assert!(access_keys.is_empty());
        let message = err.to_string();
        assert!(message.contains("Tempo session is for chain 4217"), "{message}");
        assert!(message.contains("transaction from session root"), "{message}");
        assert!(message.contains("chain 1"), "{message}");
    }

    #[test]
    fn session_access_key_is_inserted_for_session_chain() {
        let root = foundry_wallets::utils::create_private_key_signer(ROOT_PRIVATE_KEY).unwrap();
        let root_address = root.address();
        let access_key =
            foundry_wallets::utils::create_private_key_signer(ACCESS_KEY_PRIVATE_KEY).unwrap();
        let access_key_address = access_key.address();
        let session_signer = ScriptSessionSigner {
            chain: 4217,
            root_account: root_address,
            signer: access_key,
            access_key: TempoAccessKeyConfig {
                wallet_address: root_address,
                key_address: access_key_address,
                key_authorization: None,
            },
        };
        let remaining = [RemainingScriptTransaction { chain: 4217, from: root_address }];
        let mut access_keys = HashMap::default();

        insert_session_access_key_for_remaining_transactions(
            &mut access_keys,
            session_signer,
            &remaining,
        )
        .unwrap();

        let (signer, config) =
            access_keys.get(&SignerScope::new(4217, root_address)).expect("session access key");
        assert_eq!(signer.address(), access_key_address);
        assert_eq!(config.wallet_address, root_address);
        assert_eq!(config.key_address, access_key_address);
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
    fn access_key_sets_key_id_before_estimation() {
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

        sender.set_access_key_id_for_estimation();

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
}
