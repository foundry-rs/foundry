use std::{path::PathBuf, str::FromStr, time::Duration};
use url::Url;

use alloy_consensus::{SignableTransaction, Signed};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network, TransactionBuilder};
use alloy_primitives::{Address, B256};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_rpc_client::BuiltInConnectionString;
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    opts::TransactionOpts,
    utils::{LoadConfig, get_chain, maybe_print_resolved_lane, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::ProviderBuilder,
    tempo::{TEMPO_BROWSER_GAS_BUFFER, maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_config::Chain;
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use tempo_alloy::{
    TempoNetwork,
    transport::{RelayConnector, SponsorshipMode},
};
use tempo_primitives::transaction::FEE_PAYER_SIGNATURE_MARKER;

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
        if self.tx.tempo.session_id()?.is_some() {
            return self.run_generic::<TempoNetwork>(None, None).await;
        }

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
        mut pre_resolved_signer: Option<WalletSigner>,
        mut access_key: Option<TempoAccessKeyConfig>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let Self { to, mut sig, mut args, data, send_tx, mut tx, command, unlocked, force, path } =
            self;

        let has_session = tx.tempo.session_id()?.is_some();
        if has_session && unlocked {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --unlocked");
        }
        if has_session && send_tx.browser.browser {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --browser");
        }

        let print_sponsor_hash = tx.tempo.print_sponsor_hash;
        let sponsor_url = tx.tempo.sponsor_url.clone();
        let sponsor_fee_payer = tx.tempo.sponsor;
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
                    sh_status!("Aborted.")?;
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

        if has_session
            && let Some(session) = tx.tempo.session_signer_for_wallet(
                &send_tx.eth.wallet,
                get_chain(config.chain, &provider).await?.id(),
            )?
        {
            pre_resolved_signer = Some(session.signer);
            access_key = Some(session.access_key);
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
            let chain = builder.chain();
            let (mut tx, from) = if let Some(ref ak) = access_key {
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
            if let Some(fee_payer) = sponsor_fee_payer {
                resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx,
                    Some(fee_payer),
                )
                .await?;
            }
            let hash = tx
                .compute_sponsor_hash(from)
                .ok_or_else(|| eyre!("This network does not support sponsored transactions"))?;
            sh_println!("{hash:?}")?;
            return Ok(());
        }

        if let Some(ts) = expires_at {
            sh_status!("Transaction expires at unix timestamp {ts}")?;
        }

        let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);

        // --sponsor-url is only valid with a local signer (Case 4). Bail early with a clear
        // error rather than silently ignoring it in the other signing paths.
        if let Some(ref url) = sponsor_url {
            validate_sponsor_url(url)?;
            if unlocked {
                eyre::bail!("--sponsor-url cannot be combined with --unlocked");
            }
            if send_tx.browser.browser {
                eyre::bail!("--sponsor-url cannot be combined with --browser");
            }
            if access_key.is_some() {
                eyre::bail!("--sponsor-url cannot be combined with a Tempo access key");
            }
        }

        // Launch browser signer if `--browser` flag is set
        let browser = send_tx.browser.run::<N>().await?;

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

            let chain = builder.chain();
            let (mut tx_request, _) = builder.build(config.sender).await?;
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx_request,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx_request, config.sender).await?;
            }

            cast_send(
                provider,
                tx_request,
                tempo_sponsor.is_none().then_some(chain),
                None,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
                tempo_sponsor.is_none() && !config.eth_rpc_curl,
            )
            .await?;
        // Case 2:
        // Browser wallet signs and sends the transaction in one step.
        } else if let Some(browser) = browser {
            let chain = builder.chain();
            let (mut tx_request, _) =
                builder.with_browser_wallet().build(browser.address()).await?;
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
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx_request,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx_request, browser.address()).await?;
            } else {
                resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx_request,
                    Some(browser.address()),
                )
                .await?;
                maybe_print_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    Some(&tx_request),
                    None,
                )
                .await?;
            }

            let tx_hash = browser.send_transaction_via_browser(tx_request).await?;

            let cast = CastTxSender::new(&provider);
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
                .await?;
        // Case 3:
        // Tempo access key (keychain) signing. Uses `sign_with_access_key` which
        // handles the provisioning check and embeds `key_authorization` when needed.
        } else if let Some(ak) = access_key {
            let signer = match pre_resolved_signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let chain = builder.chain();
            let (mut tx_request, _) = builder.build_with_access_key(ak.wallet_address, &ak).await?;
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx_request,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx_request, ak.wallet_address).await?;
            }
            cast_send_with_access_key(
                &provider,
                tx_request,
                &signer,
                &ak,
                tempo_sponsor.is_none().then_some(chain),
                None,
                send_tx.cast_async,
                send_tx.confirmations,
                timeout,
                tempo_sponsor.is_none() && !config.eth_rpc_curl,
            )
            .await?;
        // Case 4:
        // Remote sponsor URL: sign locally, ask the sponsor service for a fee-payer signature,
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

            tx_request.set_fee_payer_signature(FEE_PAYER_SIGNATURE_MARKER);

            let wallet = EthereumWallet::from(signer);
            let default_rpc = config.get_rpc_url_or_localhost_http()?.into_owned();
            let default = BuiltInConnectionString::from_str(&default_rpc)?;
            let relay = BuiltInConnectionString::from_str(&sponsor_url)?;
            let connector =
                RelayConnector::with_config(default, relay, SponsorshipMode::SignOnly, false);
            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .wallet(wallet)
                .connect_with(&connector)
                .await?;

            cast_send(
                provider,
                tx_request,
                None,
                None,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
                false,
            )
            .await?;
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

            let chain = builder.chain();
            let (mut tx_request, _) = builder.build(&signer).await?;
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;

            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx_request,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx_request, from).await?;
            }

            let wallet = EthereumWallet::from(signer);
            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .wallet(wallet)
                .connect_provider(&provider);

            cast_send(
                provider,
                tx_request,
                tempo_sponsor.is_none().then_some(chain),
                None,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
                tempo_sponsor.is_none() && !config.eth_rpc_curl,
            )
            .await?;
        }

        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cast_send<N: Network, P: Provider<N>>(
    provider: P,
    mut tx: N::TransactionRequest,
    chain: Option<Chain>,
    fee_payer: Option<Address>,
    cast_async: bool,
    sync: bool,
    confs: u64,
    timeout: u64,
    resolve_unknown_fee_token_symbol: bool,
) -> Result<B256>
where
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    resolve_and_set_fee_token(
        resolve_unknown_fee_token_symbol.then_some(&provider),
        chain,
        &mut tx,
        fee_payer,
    )
    .await?;
    maybe_print_fee_token(
        resolve_unknown_fee_token_symbol.then_some(&provider),
        chain,
        Some(&tx),
        fee_payer,
    )
    .await?;
    let cast = CastTxSender::new(provider);

    if sync {
        // JSON envelope not supported: N::ReceiptResponse is generic over Display but not
        // Serialize; adding Serialize would ripple across all network-generic callers.
        let (tx_hash, receipt) = cast.send_sync(tx).await?;
        sh_println!("{receipt}")?;
        Ok(tx_hash)
    } else {
        let pending_tx = cast.send(tx).await?;
        let tx_hash = *pending_tx.inner().tx_hash();
        cast.print_tx_result(tx_hash, cast_async, confs, timeout).await?;
        Ok(tx_hash)
    }
}

/// Signs a transaction with a Tempo access key and sends it via `send_raw_transaction`.
///
/// Sets `from` and `key_id` on the transaction before signing, making it idempotent for txs built
/// with [`CastTxBuilder`] (fields already set) and also with sol!-bindings (fields not yet set).
///
/// NOTE: The default implementation returns an error. Only `TempoNetwork` supports this.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn cast_send_with_access_key<N: Network, P: Provider<N>>(
    provider: &P,
    mut tx: N::TransactionRequest,
    signer: &WalletSigner,
    access_key: &TempoAccessKeyConfig,
    chain: Option<Chain>,
    fee_payer: Option<Address>,
    cast_async: bool,
    confirmations: u64,
    timeout: u64,
    resolve_unknown_fee_token_symbol: bool,
) -> Result<B256>
where
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    tx.set_from(access_key.wallet_address);
    tx.set_key_id(access_key.key_address);
    resolve_and_set_fee_token(
        resolve_unknown_fee_token_symbol.then_some(provider),
        chain,
        &mut tx,
        fee_payer,
    )
    .await?;
    maybe_print_fee_token(
        resolve_unknown_fee_token_symbol.then_some(provider),
        chain,
        Some(&tx),
        fee_payer,
    )
    .await?;
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
    CastTxSender::new(provider)
        .print_tx_result(tx_hash, cast_async, confirmations, timeout)
        .await?;
    Ok(tx_hash)
}

/// Validates that a sponsor URL uses https:// (localhost/127.0.0.1 may use http://).
pub(crate) fn validate_sponsor_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw)
        .map_err(|e| eyre::eyre!("--sponsor-url is not a valid URL ({raw}): {e}"))?;

    match url.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = url.host_str().unwrap_or("");
            if host == "localhost" || host == "127.0.0.1" {
                return Ok(());
            }
            eyre::bail!(
                "--sponsor-url must use https:// for non-local endpoints (got {raw}). \
                 The sponsor relay is a trusted third party; use an encrypted channel."
            );
        }
        _ => eyre::bail!(
            "--sponsor-url must start with https:// (got {raw}). \
             The sponsor relay is a trusted third party; use an encrypted channel."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_sponsor_url() {
        // accepted
        assert!(validate_sponsor_url("https://sponsor.tempo.xyz/tp_abc").is_ok());
        assert!(validate_sponsor_url("http://localhost:8545").is_ok());
        assert!(validate_sponsor_url("http://127.0.0.1:8545").is_ok());

        // rejected
        assert!(validate_sponsor_url("http://sponsor.tempo.xyz").is_err());
        assert!(validate_sponsor_url("not-a-url").is_err());
        // bypass attempts that fooled the old starts_with check
        assert!(validate_sponsor_url("http://localhost.evil.com").is_err());
        assert!(validate_sponsor_url("http://127.0.0.1.evil.com").is_err());
    }
}
