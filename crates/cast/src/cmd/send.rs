use std::{path::PathBuf, str::FromStr, time::Duration};

use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::Encodable2718;
use alloy_ens::NameOrAddress;
use alloy_network::{
    Ethereum, EthereumWallet, Network, NetworkTransactionBuilder, TransactionBuilder,
};
use alloy_primitives::{Address, B256, hex};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
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
use serde_json::Value;
use tempo_alloy::TempoNetwork;

use crate::{
    cmd::tip20::iso4217_warning_message,
    tx::{self, CastTxBuilder, CastTxSender, SendTxOpts},
};
use tempo_contracts::precompiles::{TIP20_FACTORY_ADDRESS, is_iso4217_currency};

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
    pub async fn run(self) -> Result<()> {
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
        let sponsor_url = tx.tempo.sponsor_url.clone();
        let expires_at = tx.tempo.resolve_expires();
        let tempo_sponsor = if print_sponsor_hash || sponsor_url.is_some() {
            None
        } else {
            tx.tempo.sponsor_config().await?
        };

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

        if let Some(ts) = expires_at {
            sh_println!("Transaction expires at unix timestamp {ts}")?;
        }

        let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);

        // Launch browser signer if `--browser` flag is set
        let browser = send_tx.browser.run::<N>().await?;

        // --sponsor-url is only valid with a local signer (Case 4). Bail early with a clear
        // error rather than silently ignoring it in the other signing paths.
        if let Some(ref url) = sponsor_url {
            validate_sponsor_url(url)?;
            if unlocked {
                eyre::bail!("--sponsor-url cannot be combined with --unlocked");
            }
            if browser.is_some() {
                eyre::bail!("--sponsor-url cannot be combined with --browser");
            }
            if access_key.is_some() {
                eyre::bail!("--sponsor-url cannot be combined with a Tempo access key");
            }
        }

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
                    sh_warn!("Switching to chain {}", config_chain)?;
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
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor.attach_and_print::<N>(&mut tx_request, config.sender).await?;
            }

            cast_send(
                provider,
                tx_request,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
            )
            .await
        // Case 2:
        // Browser wallet signs and sends the transaction in one step.
        } else if let Some(browser) = browser {
            let chain = builder.chain();
            let (mut tx_request, _) = builder.build(browser.address()).await?;
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;

            // Browser wallets may sign with P256/WebAuthn instead of secp256k1, which
            // costs more gas for signature verification on Tempo chains. Add a
            // conservative buffer since we can't determine the signature type beforehand.
            if chain.is_tempo()
                && let Some(gas) = tx_request.gas_limit()
            {
                tx_request.set_gas_limit(gas + TEMPO_BROWSER_GAS_BUFFER);
            }
            if let Some(sponsor) = &tempo_sponsor {
                sponsor.attach_and_print::<N>(&mut tx_request, browser.address()).await?;
            }

            let tx_hash = browser.send_transaction_via_browser(tx_request).await?;

            let cast = CastTxSender::new(&provider);
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await
        // Case 3:
        // Tempo access key (keychain) signing. Uses `sign_with_access_key` which
        // handles the provisioning check and embeds `key_authorization` when needed.
        } else if let Some(ak) = access_key {
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let (mut tx_request, _) = builder.build_with_access_key(ak.wallet_address, &ak).await?;
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor.attach_and_print::<N>(&mut tx_request, ak.wallet_address).await?;
            }
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
        // Case 4:
        // Remote sponsor URL: sign locally, get sponsor signature from the service,
        // then submit the fully-sponsored tx to the regular RPC.
        } else if let Some(sponsor_url) = sponsor_url {
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let from = signer.address();

            tx::validate_from_address(send_tx.eth.wallet.from, from)?;

            let (mut tx_request, _) = builder.build(&signer).await?;
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;

            // Set a placeholder fee_payer_signature so encode_for_signing produces the
            // *sponsored* signing payload. The sender must commit to this variant; otherwise
            // the sponsor can't attach its real signature later without invalidating the
            // sender's hash. The sponsor service will overwrite this placeholder.
            let dummy_sponsor_sig = Signature::from_scalars_and_parity(
                B256::with_last_byte(1),
                B256::with_last_byte(1),
                false,
            );
            tx_request.set_fee_payer_signature(dummy_sponsor_sig);

            // Sign the tx locally.
            let wallet = EthereumWallet::from(signer);
            let signed_tx = tx_request.build(&wallet).await?;
            let raw_tx = hex::encode_prefixed(signed_tx.encoded_2718());

            // Send to the sponsor service to get the fee payer signature attached.
            let sponsored_raw_tx = sign_via_sponsor_url(&sponsor_url, &raw_tx).await?;

            // Submit the fully-sponsored tx via the regular RPC.
            let sponsored_bytes = hex::decode(&sponsored_raw_tx)?;
            let cast = CastTxSender::new(&provider);
            let pending = cast.send_raw(&sponsored_bytes).await?;
            let tx_hash = *pending.inner().tx_hash();
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await
        // Case 5:
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
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;

            if let Some(sponsor) = &tempo_sponsor {
                sponsor.attach_and_print::<N>(&mut tx_request, from).await?;
            }

            let wallet = EthereumWallet::from(signer);
            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .wallet(wallet)
                .connect_provider(&provider);

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

/// Validates that a sponsor URL uses https:// (localhost/127.0.0.1 may use http://).
fn validate_sponsor_url(url: &str) -> Result<()> {
    let lower = url.to_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    if lower.starts_with("http://") {
        // Allow plain http only for local testing.
        let host_part = lower.trim_start_matches("http://");
        if host_part.starts_with("localhost") || host_part.starts_with("127.0.0.1") {
            return Ok(());
        }
        eyre::bail!(
            "--sponsor-url must use https:// for non-local endpoints (got {url}). \
             The sponsor relay is a trusted third party; use an encrypted channel."
        );
    }
    eyre::bail!(
        "--sponsor-url must start with https:// (got {url}). \
         The sponsor relay is a trusted third party; use an encrypted channel."
    );
}

/// Sends a user-signed raw transaction to a remote sponsor service via JSON-RPC
/// `eth_signRawTransaction`. The service adds its fee payer signature and returns the
/// fully-sponsored raw transaction bytes, which are then ready for submission via the
/// regular RPC.
async fn sign_via_sponsor_url(url: &str, raw_tx_hex: &str) -> Result<String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_signRawTransaction",
        "params": [raw_tx_hex]
    });

    let resp = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| eyre!("sponsor service request failed: {e}"))?;

    let status = resp.status();
    let text =
        resp.text().await.map_err(|e| eyre!("failed to read sponsor service response: {e}"))?;

    if !status.is_success() {
        eyre::bail!("sponsor service returned HTTP {status}: {text}");
    }

    #[derive(serde::Deserialize)]
    struct JsonRpcResponse {
        result: Option<Value>,
        error: Option<JsonRpcError>,
    }
    // Standard JSON-RPC error object: {code, message, data?}
    #[derive(serde::Deserialize)]
    struct JsonRpcError {
        message: Option<String>,
        code: Option<i64>,
    }

    let parsed: JsonRpcResponse =
        serde_json::from_str(&text).map_err(|e| eyre!("invalid sponsor service response: {e}"))?;

    if let Some(err) = parsed.error {
        let msg = err.message.unwrap_or_else(|| format!("code {}", err.code.unwrap_or(-1)));
        eyre::bail!("sponsor service error: {msg}");
    }

    match parsed.result {
        Some(serde_json::Value::String(s)) => Ok(s),
        Some(other) => Err(eyre!("sponsor service returned unexpected result type: {other}")),
        None => Err(eyre!("sponsor service returned no result")),
    }
}
