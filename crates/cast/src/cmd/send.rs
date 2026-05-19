use std::{path::PathBuf, str::FromStr, time::Duration};

use alloy_consensus::{SignableTransaction, Signed};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256};
use alloy_provider::{
    PendingTransactionBuilder, Provider, ProviderBuilder as AlloyProviderBuilder,
};
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    json::{JsonEnvelope, print_json},
    opts::TransactionOpts,
    utils::{LoadConfig, maybe_print_resolved_lane, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::ProviderBuilder,
    tempo::TEMPO_BROWSER_GAS_BUFFER,
};
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use serde::Serialize;
use tempo_alloy::TempoNetwork;

use crate::{
    cmd::tip20::iso4217_warning_message,
    tx::{self, CastTxBuilder, CastTxSender, SendTxOpts},
};
use tempo_contracts::precompiles::{TIP20_FACTORY_ADDRESS, is_iso4217_currency};

/// `cast send --machine` payload.
#[derive(Clone, Debug, Serialize)]
struct SendData {
    broadcast: bool,
    tx_hash: String,
    from: String,
    to: Option<String>,
    contract_address: Option<String>,
    status: Option<bool>,
    block_number: Option<String>,
    gas_used: Option<String>,
    effective_gas_price: Option<String>,
}

impl SendData {
    /// Payload for `--async`: only the submitted transaction hash is known.
    fn async_only(tx_hash: B256, from: Address, to: Option<Address>) -> Self {
        Self {
            broadcast: true,
            tx_hash: format!("{tx_hash:#x}"),
            from: from.to_string(),
            to: to.map(|a| a.to_string()),
            contract_address: None,
            status: None,
            block_number: None,
            gas_used: None,
            effective_gas_price: None,
        }
    }

    /// Payload built from an observed receipt.
    fn from_receipt<N: Network>(
        receipt: &N::ReceiptResponse,
        from: Address,
        to: Option<Address>,
    ) -> Self {
        Self {
            broadcast: true,
            tx_hash: format!("{:#x}", receipt.transaction_hash()),
            from: from.to_string(),
            to: to.map(|a| a.to_string()),
            contract_address: receipt.contract_address().map(|a| a.to_string()),
            status: Some(receipt.status()),
            block_number: receipt.block_number().map(|b| b.to_string()),
            gas_used: Some(receipt.gas_used().to_string()),
            effective_gas_price: Some(receipt.effective_gas_price().to_string()),
        }
    }
}

/// CLI arguments for `cast send`.
#[derive(Debug, Parser)]
pub struct SendTxArgs {
    /// The destination of the transaction.
    ///
    /// If not provided, you must use cast send --create.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(allow_negative_numbers = true)]
    args: Vec<String>,

    /// Raw hex-encoded data for the transaction. Used instead of \[SIG\] and \[ARGS\].
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    #[command(flatten)]
    send_tx: SendTxOpts,

    #[command(subcommand)]
    command: Option<SendTxSubcommands>,

    /// Send via `eth_sendTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    unlocked: bool,

    /// Skip confirmation prompts (e.g. non-ISO 4217 currency warnings).
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    /// The path of blob data to be sent.
    #[arg(
        long,
        value_name = "BLOB_DATA_PATH",
        conflicts_with = "legacy",
        requires = "blob",
        help_heading = "Transaction options"
    )]
    path: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub enum SendTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[command(name = "--create")]
    Create {
        /// The bytecode of the contract to deploy.
        code: String,

        /// The signature of the function to call.
        sig: Option<String>,

        /// The arguments of the function to call.
        #[arg(allow_negative_numbers = true)]
        args: Vec<String>,
    },
}

impl SendTxArgs {
    /// Rejects flags whose stdout shape conflicts with the envelope contract.
    pub fn reject_machine_unsupported_flags(&self) -> Result<()> {
        if !foundry_cli::is_machine() {
            return Ok(());
        }
        let unsupported = [
            ("--browser", self.send_tx.browser.browser),
            ("--tempo.print-sponsor-hash", self.tx.tempo.print_sponsor_hash),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect::<Vec<_>>();
        if !unsupported.is_empty() {
            foundry_cli::machine::bail_machine_usage(format!(
                "`cast send` under `--machine` does not yet support {}; \
                 run without `--machine` or omit those flags.",
                unsupported.join(", ")
            ));
        }
        Ok(())
    }

    pub async fn run(self) -> Result<()> {
        self.reject_machine_unsupported_flags()?;

        // Resolve the signer early so we know if it's a Tempo access key.
        let (signer, tempo_access_key) = self.send_tx.eth.wallet.maybe_signer().await?;

        if tempo_access_key.is_some() || self.tx.tempo.is_tempo() {
            self.run_generic::<TempoNetwork>(signer, tempo_access_key).await
        } else {
            self.run_generic::<Ethereum>(signer, None).await
        }
    }

    pub async fn run_generic<N: Network>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        access_key: Option<TempoAccessKeyConfig>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let Self { to, mut sig, mut args, data, send_tx, mut tx, command, unlocked, force, path } =
            self;

        let print_sponsor_hash = tx.tempo.print_sponsor_hash;
        let expires_at = tx.tempo.resolve_expires();
        let tempo_sponsor =
            if print_sponsor_hash { None } else { tx.tempo.sponsor_config().await? };

        let blob_data = if let Some(path) = path { Some(std::fs::read(path)?) } else { None };

        if let Some(data) = data {
            sig = Some(data);
        }

        let code = if let Some(SendTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && !tx.auth.is_empty() {
                return Err(eyre!(
                    "EIP-7702 transactions can't be CREATE transactions and require a destination address"
                ));
            }
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && blob_data.is_some() {
                return Err(eyre!(
                    "EIP-4844 transactions can't be CREATE transactions and require a destination address"
                ));
            }

            sig = constructor_sig;
            args = constructor_args;
            Some(code)
        } else {
            None
        };

        // Validate ISO 4217 currency code for TIP20Factory createToken calls.
        if let Some(ref to_addr) = to {
            let is_factory = match to_addr {
                NameOrAddress::Address(addr) => *addr == TIP20_FACTORY_ADDRESS,
                NameOrAddress::Name(name) => {
                    Address::from_str(name).ok() == Some(TIP20_FACTORY_ADDRESS)
                }
            };

            if !force
                && is_factory
                && let Some(ref sig_str) = sig
                && sig_str.starts_with("createToken")
                && let Some(currency) = args.get(2)
                && !is_iso4217_currency(currency)
            {
                if foundry_cli::is_machine() {
                    foundry_cli::machine::bail_machine_usage(
                        "`cast send` would prompt to confirm a non-ISO4217 currency code; \
                         pass `--force` to skip the prompt under `--machine`.",
                    );
                }
                sh_warn!("{}", iso4217_warning_message(currency))?;
                let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
                if !matches!(response.trim(), "y" | "Y") {
                    sh_println!("Aborted.")?;
                    return Ok(());
                }
            }
        }

        let config = send_tx.eth.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;

        let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;

        if let Some(interval) = send_tx.poll_interval {
            provider.client().set_poll_interval(Duration::from_secs(interval))
        }

        // Inject access key ID into TempoOpts so it's set before gas estimation.
        if let Some(ref ak) = access_key {
            tx.tempo.key_id = Some(ak.key_address);
        }

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        // If --tempo.print-sponsor-hash was passed, build the tx, print the hash, and exit.
        if print_sponsor_hash {
            let (tx, from) = if let Some(ref ak) = access_key {
                let (tx, _) = builder.build_with_access_key(ak.wallet_address, ak).await?;
                (tx, ak.wallet_address)
            } else {
                // Use the pre-resolved signer to derive the actual sender address, since the
                // sponsor hash commits to the sender.
                let signer = pre_resolved_signer.as_ref().ok_or_else(|| {
                    eyre!("--tempo.print-sponsor-hash requires a signer (e.g. --private-key)")
                })?;
                let from = signer.address();
                let (tx, _) = builder.build(from).await?;
                (tx, from)
            };
            let hash = tx
                .compute_sponsor_hash(from)
                .ok_or_else(|| eyre!("This network does not support sponsored transactions"))?;
            sh_println!("{hash:?}")?;
            return Ok(());
        }

        if let Some(ts) = expires_at
            && !foundry_cli::is_machine()
        {
            sh_println!("Transaction expires at unix timestamp {ts}")?;
        }

        let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);

        // Launch browser signer if `--browser` flag is set
        let browser = send_tx.browser.run::<N>().await?;

        let machine = foundry_cli::is_machine();

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked && browser.is_none() {
            // only check current chain id if it was specified in the config
            if let Some(config_chain) = config.chain {
                let current_chain_id = provider.get_chain_id().await?;
                let config_chain_id = config_chain.id();
                // switch chain if current chain id is not the same as the one specified in the
                // config
                if config_chain_id != current_chain_id {
                    if !machine {
                        sh_warn!("Switching to chain {}", config_chain)?;
                    }
                    provider
                        .raw_request::<_, ()>(
                            "wallet_switchEthereumChain".into(),
                            [serde_json::json!({
                                "chainId": format!("0x{:x}", config_chain_id),
                            })],
                        )
                        .await?;
                }
            }

            let (mut tx_request, _) = builder.build(config.sender).await?;
            machine_or_print_lane::<N>(machine, resolved_lane.as_ref(), &tx_request)?;
            attach_sponsor::<N>(machine, &tempo_sponsor, &mut tx_request, config.sender).await?;

            if machine {
                let to = tx_request.to();
                machine_send_via_provider::<N, _>(
                    &provider,
                    tx_request,
                    config.sender,
                    to,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    timeout,
                )
                .await
            } else {
                cast_send(
                    provider,
                    tx_request,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    timeout,
                )
                .await
            }
        // Case 2:
        // Browser wallet signs and sends the transaction in one step.
        } else if let Some(browser) = browser {
            let chain = builder.chain();
            let (mut tx_request, _) = builder.build(browser.address()).await?;
            machine_or_print_lane::<N>(machine, resolved_lane.as_ref(), &tx_request)?;

            // Browser wallets may sign with P256/WebAuthn instead of secp256k1, which
            // costs more gas for signature verification on Tempo chains. Add a
            // conservative buffer since we can't determine the signature type beforehand.
            if chain.is_tempo()
                && let Some(gas) = tx_request.gas_limit()
            {
                tx_request.set_gas_limit(gas + TEMPO_BROWSER_GAS_BUFFER);
            }
            attach_sponsor::<N>(machine, &tempo_sponsor, &mut tx_request, browser.address())
                .await?;

            let to = tx_request.to();
            let tx_hash = browser.send_transaction_via_browser(tx_request).await?;

            if machine {
                machine_send_after_tx_hash::<N, _>(
                    &provider,
                    tx_hash,
                    browser.address(),
                    to,
                    send_tx.cast_async,
                    send_tx.confirmations,
                    timeout,
                )
                .await
            } else {
                let cast = CastTxSender::new(&provider);
                cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
                    .await
            }
        // Case 3:
        // Tempo access key (keychain) signing. Uses `sign_with_access_key` which
        // handles the provisioning check and embeds `key_authorization` when needed.
        } else if let Some(ak) = access_key {
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let (mut tx_request, _) = builder.build_with_access_key(ak.wallet_address, &ak).await?;
            machine_or_print_lane::<N>(machine, resolved_lane.as_ref(), &tx_request)?;
            attach_sponsor::<N>(machine, &tempo_sponsor, &mut tx_request, ak.wallet_address)
                .await?;

            if machine {
                let to = tx_request.to();
                machine_send_with_access_key::<N, _>(
                    &provider,
                    tx_request,
                    &signer,
                    &ak,
                    to,
                    send_tx.cast_async,
                    send_tx.confirmations,
                    timeout,
                )
                .await
            } else {
                cast_send_with_access_key(
                    &provider,
                    tx_request,
                    &signer,
                    &ak,
                    send_tx.cast_async,
                    send_tx.confirmations,
                    timeout,
                )
                .await
            }
        // Case 4:
        // An option to use a local signer was provided.
        // If we cannot successfully instantiate a local signer, then we will assume we don't have
        // enough information to sign and we must bail.
        } else {
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let from = signer.address();

            tx::validate_from_address(send_tx.eth.wallet.from, from)?;

            let (mut tx_request, _) = builder.build(&signer).await?;
            machine_or_print_lane::<N>(machine, resolved_lane.as_ref(), &tx_request)?;
            attach_sponsor::<N>(machine, &tempo_sponsor, &mut tx_request, from).await?;

            let wallet = EthereumWallet::from(signer);
            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .wallet(wallet)
                .connect_provider(&provider);

            if machine {
                let to = tx_request.to();
                machine_send_via_provider::<N, _>(
                    &provider,
                    tx_request,
                    from,
                    to,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    timeout,
                )
                .await
            } else {
                cast_send(
                    provider,
                    tx_request,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    timeout,
                )
                .await
            }
        }
    }
}

pub(crate) async fn cast_send<N: Network, P: Provider<N>>(
    provider: P,
    tx: N::TransactionRequest,
    cast_async: bool,
    sync: bool,
    confs: u64,
    timeout: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    let cast = CastTxSender::new(provider);

    if sync {
        // Send transaction and wait for receipt synchronously
        let receipt = cast.send_sync(tx).await?;
        sh_println!("{receipt}")?;
    } else {
        let pending_tx = cast.send(tx).await?;
        let tx_hash = *pending_tx.inner().tx_hash();
        cast.print_tx_result(tx_hash, cast_async, confs, timeout).await?;
    }

    Ok(())
}

/// Signs a transaction with a Tempo access key and sends it via `send_raw_transaction`.
///
/// Sets `from` and `key_id` on the transaction before signing, making it idempotent for txs built
/// with [`CastTxBuilder`] (fields already set) and also with sol!-bindings (fields not yet set).
///
/// NOTE: The default implementation returns an error. Only `TempoNetwork` supports this.
pub(crate) async fn cast_send_with_access_key<N: Network, P: Provider<N>>(
    provider: &P,
    mut tx: N::TransactionRequest,
    signer: &WalletSigner,
    access_key: &TempoAccessKeyConfig,
    cast_async: bool,
    confirmations: u64,
    timeout: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    tx.set_from(access_key.wallet_address);
    tx.set_key_id(access_key.key_address);
    let raw_tx = tx
        .sign_with_access_key(
            provider,
            signer,
            access_key.wallet_address,
            access_key.key_address,
            access_key.key_authorization.as_ref(),
        )
        .await?;
    let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();
    CastTxSender::new(provider).print_tx_result(tx_hash, cast_async, confirmations, timeout).await
}

/// Print the resolved lane preview unless `--machine` owns stdout/stderr.
fn machine_or_print_lane<N: Network>(
    machine: bool,
    resolved_lane: Option<&foundry_cli::utils::ResolvedLane>,
    tx_request: &N::TransactionRequest,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if machine {
        return Ok(());
    }
    maybe_print_resolved_lane(resolved_lane, tx_request.nonce().unwrap_or_default())
}

/// Attach Tempo sponsorship, choosing the silent variant under `--machine`.
async fn attach_sponsor<N: Network>(
    machine: bool,
    tempo_sponsor: &Option<foundry_common::tempo::TempoSponsor>,
    tx_request: &mut N::TransactionRequest,
    sender: Address,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    let Some(sponsor) = tempo_sponsor else { return Ok(()) };
    if machine {
        sponsor.attach_silent::<N>(tx_request, sender).await?;
    } else {
        sponsor.attach_and_print::<N>(tx_request, sender).await?;
    }
    Ok(())
}

/// `--machine` send via `provider.send_transaction[_sync]`. Captures
/// `tx_hash` after broadcast so receipt-wait failures preserve it.
#[expect(clippy::too_many_arguments)]
async fn machine_send_via_provider<N: Network, P: Provider<N>>(
    provider: &P,
    tx: N::TransactionRequest,
    from: Address,
    to: Option<Address>,
    cast_async: bool,
    sync: bool,
    confs: u64,
    timeout: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if sync {
        let receipt_result: Result<N::ReceiptResponse> =
            provider.send_transaction_sync(tx).await.map_err(Into::into);
        return finish_machine_send::<N>(receipt_result.map(|r| (r, from, to)), None, from, to);
    }
    let pending_tx = match provider.send_transaction(tx).await {
        Ok(p) => p,
        Err(e) => bail_machine_send_error(e.into(), None, from, to),
    };
    let tx_hash = *pending_tx.inner().tx_hash();
    if cast_async {
        return emit_machine_send_success(SendData::async_only(tx_hash, from, to));
    }
    let receipt_result: Result<N::ReceiptResponse> = pending_tx
        .with_required_confirmations(confs)
        .with_timeout(Some(Duration::from_secs(timeout)))
        .get_receipt()
        .await
        .map_err(Into::into);
    finish_machine_send::<N>(receipt_result.map(|r| (r, from, to)), Some(tx_hash), from, to)
}

/// `--machine` send path for already-broadcast transactions (browser flow).
async fn machine_send_after_tx_hash<N: Network, P: Provider<N>>(
    provider: &P,
    tx_hash: B256,
    from: Address,
    to: Option<Address>,
    cast_async: bool,
    confs: u64,
    timeout: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if cast_async {
        return emit_machine_send_success(SendData::async_only(tx_hash, from, to));
    }
    let receipt_result: Result<N::ReceiptResponse> =
        PendingTransactionBuilder::<N>::new(provider.root().clone(), tx_hash)
            .with_required_confirmations(confs)
            .with_timeout(Some(Duration::from_secs(timeout)))
            .get_receipt()
            .await
            .map_err(Into::into);
    finish_machine_send::<N>(receipt_result.map(|r| (r, from, to)), Some(tx_hash), from, to)
}

/// `--machine` send path for Tempo access-key signing.
#[expect(clippy::too_many_arguments)]
async fn machine_send_with_access_key<N: Network, P: Provider<N>>(
    provider: &P,
    mut tx: N::TransactionRequest,
    signer: &WalletSigner,
    access_key: &TempoAccessKeyConfig,
    to: Option<Address>,
    cast_async: bool,
    confs: u64,
    timeout: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    let from = access_key.wallet_address;
    tx.set_from(from);
    tx.set_key_id(access_key.key_address);
    let raw_tx = match tx
        .sign_with_access_key(
            provider,
            signer,
            access_key.wallet_address,
            access_key.key_address,
            access_key.key_authorization.as_ref(),
        )
        .await
    {
        Ok(raw) => raw,
        Err(e) => bail_machine_send_error(e, None, from, to),
    };
    let pending_tx = match provider.send_raw_transaction(&raw_tx).await {
        Ok(p) => p,
        Err(e) => bail_machine_send_error(e.into(), None, from, to),
    };
    let tx_hash = *pending_tx.tx_hash();
    if cast_async {
        return emit_machine_send_success(SendData::async_only(tx_hash, from, to));
    }
    let receipt_result: Result<N::ReceiptResponse> = pending_tx
        .with_required_confirmations(confs)
        .with_timeout(Some(Duration::from_secs(timeout)))
        .get_receipt()
        .await
        .map_err(Into::into);
    finish_machine_send::<N>(receipt_result.map(|r| (r, from, to)), Some(tx_hash), from, to)
}

/// Emit a success envelope, or reclassify a reverted receipt as
/// `chain.broadcast_failed` with the receipt fields kept in `details`.
fn finish_machine_send<N: Network>(
    receipt: Result<(N::ReceiptResponse, Address, Option<Address>)>,
    known_tx_hash: Option<B256>,
    from: Address,
    to: Option<Address>,
) -> Result<()> {
    match receipt {
        Ok((receipt, rfrom, rto)) => {
            let data = SendData::from_receipt::<N>(&receipt, rfrom, rto);
            if data.status == Some(false) {
                // Reverted: keep receipt fields so agents can debug.
                let details = serde_json::json!({
                    "tx_hash": data.tx_hash,
                    "from": data.from,
                    "to": data.to,
                    "contract_address": data.contract_address,
                    "block_number": data.block_number,
                    "gas_used": data.gas_used,
                    "effective_gas_price": data.effective_gas_price,
                    "broadcast": true,
                    "receipt_observed": true,
                });
                foundry_cli::machine::bail_machine_diagnostic_with_details(
                    foundry_cli::diagnostic::chain::BROADCAST_FAILED,
                    foundry_cli::exit_code::ExitCode::GenericError,
                    format!("transaction reverted (receipt status 0): {}", data.tx_hash),
                    details,
                );
            }
            emit_machine_send_success(data)
        }
        Err(e) => bail_machine_send_error(e, known_tx_hash, from, to),
    }
}

/// Emit a successful `cast send` envelope on stdout.
fn emit_machine_send_success(data: SendData) -> Result<()> {
    print_json(&JsonEnvelope::success(data))?;
    Ok(())
}

/// Bail with a typed error envelope. When `known_tx_hash` is `Some`, the
/// broadcast happened; preserve the hash in `details`.
fn bail_machine_send_error(
    err: eyre::Report,
    known_tx_hash: Option<B256>,
    from: Address,
    to: Option<Address>,
) -> ! {
    let code = classify_send_error(&err);
    let message = format!("{err:#}");
    match known_tx_hash {
        Some(tx_hash) => {
            let details = serde_json::json!({
                "tx_hash": format!("{tx_hash:#x}"),
                "from": from.to_string(),
                "to": to.map(|a| a.to_string()),
                "broadcast": true,
                "receipt_observed": false,
            });
            foundry_cli::machine::bail_machine_diagnostic_with_details(
                code,
                foundry_cli::exit_code::ExitCode::GenericError,
                message,
                details,
            )
        }
        None => foundry_cli::machine::bail_machine_diagnostic(
            code,
            foundry_cli::exit_code::ExitCode::GenericError,
            message,
        ),
    }
}

/// Map a send failure's cause chain to a typed diagnostic code, falling
/// back to `chain.broadcast_failed` when no keyword matches.
fn classify_send_error(err: &eyre::Report) -> &'static str {
    use std::fmt::Write;
    let mut buf = String::new();
    for cause in err.chain() {
        let _ = writeln!(buf, "{cause}");
    }
    let lower = buf.to_lowercase();

    if lower.contains("timed out") || lower.contains("timeout") {
        return foundry_cli::diagnostic::network::RPC_TIMEOUT;
    }
    if lower.contains("unauthorized") || lower.contains("401") {
        return foundry_cli::diagnostic::network::RPC_UNAUTHORIZED;
    }
    if (lower.contains("signature") || lower.contains("sign"))
        && (lower.contains("reject") || lower.contains("denied"))
    {
        return foundry_cli::diagnostic::wallet::SIGNATURE_REJECTED;
    }
    foundry_cli::diagnostic::chain::BROADCAST_FAILED
}
