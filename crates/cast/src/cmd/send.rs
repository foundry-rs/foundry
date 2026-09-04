use std::{path::PathBuf, str::FromStr, time::Duration};
use url::Url;

use alloy_consensus::{SignableTransaction, Signed};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network, TransactionBuilder};
use alloy_primitives::{Address, B256};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::{Signature, Signer};
#[cfg(feature = "base")]
use base_common_network::Base as BaseNetwork;
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
    tempo::{maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_config::Chain;
use foundry_wallets::{TempoAccountsWallet, WalletSigner};
use tempo_alloy::TempoNetwork;
use tempo_primitives::transaction::FEE_PAYER_SIGNATURE_MARKER;

#[cfg(feature = "base")]
use crate::cmd::resolve_network;
use crate::{
    cmd::{auth::confirm_auth_rpc_disclosure_during_build, tip20::iso4217_warning_message},
    tempo,
    tx::{self, CastTxBuilder, CastTxSender, SendTxOpts, TxParams},
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

    /// Raw hex-encoded data for the transaction. Used instead of `SIG` and `ARGS`.
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

    /// Relative percentage to multiply the gas estimate by.
    #[arg(long, value_name = "PERCENT", help_heading = "Transaction options")]
    gas_estimate_multiplier: Option<u64>,

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
    /// Creates a `cast send` invocation for pre-encoded contract calldata.
    pub(crate) fn contract_call(
        to: NameOrAddress,
        data: String,
        send_tx: SendTxOpts,
        tx: TxParams,
    ) -> Self {
        Self {
            to: Some(to),
            sig: None,
            args: Vec::new(),
            data: Some(data),
            send_tx,
            command: None,
            unlocked: false,
            force: false,
            gas_estimate_multiplier: None,
            tx: tx.into_transaction_opts(),
            path: None,
        }
    }

    pub async fn run(self) -> Result<()> {
        if self.tx.tempo.session_id()?.is_some() {
            return self.run_generic::<TempoNetwork>(None, None).await;
        }

        let (is_tempo, signer, tempo_access_key) =
            tempo::resolve_transaction_network_and_signer(&self.tx.tempo, &self.send_tx.eth)
                .await?;

        if is_tempo {
            return self.run_generic::<TempoNetwork>(signer, tempo_access_key).await;
        }

        #[cfg(feature = "base")]
        if resolve_network(&self.send_tx.eth.load_config()?).await?.is_base() {
            return self.run_generic::<BaseNetwork>(signer, None).await;
        }

        self.run_generic::<Ethereum>(signer, None).await
    }

    pub async fn run_generic<N: Network>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        mut access_key: Option<TempoAccountsWallet>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let Self {
            to,
            mut sig,
            mut args,
            data,
            send_tx,
            mut tx,
            command,
            unlocked,
            force,
            gas_estimate_multiplier,
            path,
        } = self;

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

        if has_session || access_key.is_some() {
            let chain = get_chain(config.chain, &provider).await?;
            let (_, resolved_access_key) =
                tempo::resolve_session_or_wallet_signer(&tx.tempo, &send_tx.eth.wallet, chain.id())
                    .await?;
            access_key = resolved_access_key;
        }

        // Inject access key ID into TempoOpts so it's set before gas estimation.
        if let Some(ref ak) = access_key {
            tx.tempo.key_id = Some(ak.key_id()?);
        }

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_gas_estimate_multiplier(gas_estimate_multiplier)
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        // If --tempo.print-sponsor-hash was passed, build the tx, print the hash, and exit.
        if print_sponsor_hash {
            let chain = builder.chain();
            let (mut tx, from) = if let Some(ref ak) = access_key {
                if !confirm_auth_rpc_disclosure_during_build(&builder, ak.account(), force)? {
                    return Ok(());
                }
                let (tx, _, prepared) = builder.build_with_tempo_wallet(ak).await?;
                (tx, prepared.account())
            } else {
                // Use the pre-resolved signer to derive the actual sender address, since the
                // sponsor hash commits to the sender.
                let signer = pre_resolved_signer.as_ref().ok_or_else(|| {
                    eyre!("--tempo.print-sponsor-hash requires a signer (e.g. --private-key)")
                })?;
                let from = signer.address();
                if !confirm_auth_rpc_disclosure_during_build(&builder, signer, force)? {
                    return Ok(());
                }
                let (tx, _) = builder.build(signer).await?;
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

        // --sponsor-url is valid with local signers and Tempo access keys. Bail early rather than
        // silently ignoring it in signing paths that cannot produce a raw transaction locally.
        if let Some(ref url) = sponsor_url {
            validate_sponsor_url(url)?;
            if unlocked {
                eyre::bail!("--sponsor-url cannot be combined with --unlocked");
            }
            if send_tx.browser.browser {
                eyre::bail!("--sponsor-url cannot be combined with --browser");
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
            if !confirm_auth_rpc_disclosure_during_build(&builder, config.sender, force)? {
                return Ok(());
            }
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
            if !confirm_auth_rpc_disclosure_during_build(&builder, browser.address(), force)? {
                return Ok(());
            }
            let (mut tx_request, _) =
                builder.with_browser_wallet().build(browser.address()).await?;
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
                sponsor.attach_and_print::<N>(&mut tx_request, browser.address()).await?;
            } else {
                let fee_token = resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx_request,
                    Some(browser.address()),
                )
                .await?;
                maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), fee_token)
                    .await?;
            }

            if chain.id() != browser.chain_id() {
                sh_warn!("Switching browser wallet to chain {}", chain)?;
                browser.switch_chain(chain.id()).await?;
            }

            let tx_hash = browser.send_transaction_via_browser(tx_request).await?;

            let cast = CastTxSender::new(&provider);
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
                .await?;
        // Case 3: Tempo access-key wallet.
        } else if let Some(ak) = access_key {
            let chain = builder.chain();
            if !confirm_auth_rpc_disclosure_during_build(&builder, ak.account(), force)? {
                return Ok(());
            }
            let (mut tx_request, _, prepared) = builder.build_with_tempo_wallet(&ak).await?;
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
                sponsor.attach_and_print::<N>(&mut tx_request, prepared.account()).await?;
            }
            if let Some(sponsor_url) = sponsor_url.as_deref() {
                cast_send_with_tempo_wallet_via_sponsor(
                    &provider,
                    tx_request,
                    &prepared,
                    sponsor_url,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    timeout,
                )
                .await?;
            } else {
                cast_send_with_tempo_wallet(
                    &provider,
                    tx_request,
                    &prepared,
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

            if !confirm_auth_rpc_disclosure_during_build(&builder, &signer, force)? {
                return Ok(());
            }
            let (mut tx_request, _) = builder.build(&signer).await?;
            maybe_print_resolved_lane(
                resolved_lane.as_ref(),
                tx_request.nonce().unwrap_or_default(),
            )?;

            tx_request.set_fee_payer_signature(FEE_PAYER_SIGNATURE_MARKER);

            let wallet = EthereumWallet::from(signer);
            let connector = tempo::sponsor_relay_connector(&provider, &sponsor_url)?;
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
            if !confirm_auth_rpc_disclosure_during_build(&builder, &signer, force)? {
                return Ok(());
            }
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
    let fee_token = resolve_and_set_fee_token(
        resolve_unknown_fee_token_symbol.then_some(&provider),
        chain,
        &mut tx,
        fee_payer,
    )
    .await?;
    maybe_print_fee_token(resolve_unknown_fee_token_symbol.then_some(&provider), fee_token).await?;
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

/// Sends a raw transaction using the RPC method selected by `sync`.
pub(crate) async fn cast_send_raw<N: Network, P: Provider<N>>(
    provider: &P,
    raw_tx: &[u8],
    sync: bool,
) -> Result<(B256, Option<String>)>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    if sync {
        let (tx_hash, receipt) = CastTxSender::new(provider).send_raw_sync(raw_tx).await?;
        Ok((tx_hash, Some(receipt)))
    } else {
        let tx_hash = *provider.send_raw_transaction(raw_tx).await?.tx_hash();
        Ok((tx_hash, None))
    }
}

/// Signs a prepared transaction with a Tempo wallet and sends it as a raw transaction.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn cast_send_with_tempo_wallet<N: Network, P: Provider<N>>(
    provider: &P,
    mut tx: N::TransactionRequest,
    wallet: &TempoAccountsWallet,
    chain: Option<Chain>,
    fee_payer: Option<Address>,
    cast_async: bool,
    sync: bool,
    confirmations: u64,
    timeout: u64,
    resolve_unknown_fee_token_symbol: bool,
) -> Result<B256>
where
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    let fee_token = resolve_and_set_fee_token(
        resolve_unknown_fee_token_symbol.then_some(provider),
        chain,
        &mut tx,
        fee_payer,
    )
    .await?;
    maybe_print_fee_token(resolve_unknown_fee_token_symbol.then_some(provider), fee_token).await?;
    let raw_tx = tx.sign_with_tempo_wallet(wallet).await?;
    let cast = CastTxSender::new(provider);
    let (tx_hash, receipt) = cast_send_raw(provider, &raw_tx, sync).await?;

    if let Some(receipt) = receipt {
        sh_println!("{receipt}")?;
    } else {
        cast.print_tx_result(tx_hash, cast_async, confirmations, timeout).await?;
    }
    Ok(tx_hash)
}

/// Signs a prepared transaction with a Tempo wallet, obtains a remote fee-payer signature, and
/// broadcasts the sponsored transaction through the original provider transport.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn cast_send_with_tempo_wallet_via_sponsor<N: Network, P: Provider<N>>(
    provider: &P,
    mut tx: N::TransactionRequest,
    wallet: &TempoAccountsWallet,
    sponsor_url: &str,
    cast_async: bool,
    sync: bool,
    confirmations: u64,
    timeout: u64,
) -> Result<B256>
where
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    tx.set_fee_payer_signature(FEE_PAYER_SIGNATURE_MARKER);
    let connector = tempo::sponsor_relay_connector(provider, sponsor_url)?;
    let sponsor_provider =
        AlloyProviderBuilder::<_, _, N>::default().connect_with(&connector).await?;
    cast_send_with_tempo_wallet(
        &sponsor_provider,
        tx,
        wallet,
        None,
        None,
        cast_async,
        sync,
        confirmations,
        timeout,
        false,
    )
    .await
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
        _ => {
            eyre::bail!(
                "--sponsor-url must start with https:// (got {raw}). \
             The sponsor relay is a trusted third party; use an encrypted channel."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::{RequestPacket, ResponsePacket};
    use alloy_provider::mock::Asserter;
    use alloy_rpc_client::RpcClient;
    use alloy_rpc_types::TransactionRequest;
    use alloy_transport::{TransportError, TransportFut, mock::MockTransport};
    use foundry_wallets::utils::create_local_signer;
    use std::{
        sync::{Arc, Mutex},
        task::{Context, Poll},
    };
    use tempo_alloy::rpc::TempoTransactionRequest;
    use tower::Service;

    #[derive(Clone)]
    struct RecordingTransport {
        inner: MockTransport,
        methods: Arc<Mutex<Vec<String>>>,
    }

    impl Service<RequestPacket> for RecordingTransport {
        type Response = ResponsePacket;
        type Error = TransportError;
        type Future = TransportFut<'static>;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.inner.poll_ready(cx)
        }

        fn call(&mut self, req: RequestPacket) -> Self::Future {
            if let RequestPacket::Single(req) = &req {
                self.methods.lock().unwrap().push(req.method().to_string());
            }
            self.inner.call(req)
        }
    }

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

    #[test]
    fn parses_gas_estimate_multiplier() {
        let args = SendTxArgs::parse_from([
            "cast-send",
            "0x0000000000000000000000000000000000000000",
            "--gas-estimate-multiplier",
            "125",
        ]);
        assert_eq!(args.gas_estimate_multiplier, Some(125));
    }

    #[tokio::test]
    async fn tempo_wallet_sync_send_uses_sync_rpc_method() {
        let asserter = Asserter::new();
        let tx_hash = B256::repeat_byte(0x11);
        asserter.push_success(&serde_json::json!({
            "type": "0x76",
            "status": "0x1",
            "cumulativeGasUsed": "0x5208",
            "logs": [],
            "logsBloom": format!("0x{}", "00".repeat(256)),
            "transactionHash": tx_hash,
            "transactionIndex": "0x0",
            "blockHash": B256::repeat_byte(0x22),
            "blockNumber": "0x1",
            "gasUsed": "0x5208",
            "effectiveGasPrice": "0x1",
            "from": Address::ZERO,
            "to": Address::ZERO,
            "contractAddress": null,
            "feeToken": Address::repeat_byte(0x55),
            "feePayer": Address::ZERO,
        }));
        let methods = Arc::new(Mutex::new(Vec::new()));
        let transport =
            RecordingTransport { inner: MockTransport::new(asserter), methods: methods.clone() };
        let provider = AlloyProviderBuilder::new_with_network::<TempoNetwork>()
            .connect_client(RpcClient::new(transport, true));
        let root = Address::repeat_byte(0x33);
        let access_key = create_local_signer(
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
        )
        .unwrap();
        let wallet =
            TempoAccountsWallet::from_secp256k1(root, access_key, None).with_chain_id(4217);
        let tx = TempoTransactionRequest {
            inner: TransactionRequest {
                to: Some(Address::repeat_byte(0x44).into()),
                nonce: Some(0),
                gas: Some(100_000),
                max_fee_per_gas: Some(1),
                max_priority_fee_per_gas: Some(1),
                chain_id: Some(4217),
                ..Default::default()
            },
            ..Default::default()
        };

        let actual_hash = cast_send_with_tempo_wallet(
            &provider, tx, &wallet, None, None, false, true, 1, 1, false,
        )
        .await
        .unwrap();

        assert_eq!(actual_hash, tx_hash);
        assert_eq!(methods.lock().unwrap().as_slice(), ["eth_sendRawTransactionSync"]);
    }
}
