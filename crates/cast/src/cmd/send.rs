use crate::{
    cmd::{
        auth::{confirm_and_build, confirm_and_build_with_tempo_wallet},
        confirm_continue,
        tip20::iso4217_warning_message,
    },
    tempo,
    tx::{self, CastTxBuilder, CastTxSender, SendTxOpts, TxParams, apply_poll_interval},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network};
use alloy_primitives::{Address, B256, hex};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    opts::TransactionOpts,
    utils::{LoadConfig, get_chain, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::ProviderBuilder,
};
use foundry_config::{Chain, Config};
use foundry_wallets::{TempoAccountsWallet, WalletSigner, wallet_browser::signer::BrowserSigner};
use std::{path::PathBuf, str::FromStr};
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{TIP20_FACTORY_ADDRESS, is_iso4217_currency};
use tempo_primitives::transaction::FEE_PAYER_SIGNATURE_MARKER;
use url::Url;

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
        data: Vec<u8>,
        send_tx: SendTxOpts,
        tx: TxParams,
    ) -> Self {
        Self {
            to: Some(to),
            sig: None,
            args: Vec::new(),
            data: Some(hex::encode_prefixed(data)),
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
            self.run_generic::<TempoNetwork>(signer, tempo_access_key).await
        } else {
            self.run_generic::<Ethereum>(signer, None).await
        }
    }

    /// Runs a contract call with an already resolved browser signer.
    pub(crate) async fn run_generic_with_browser<N: Network>(
        self,
        browser: BrowserSigner<N>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        self.run_generic_inner::<N>(None, None, Some(browser)).await
    }

    pub(crate) async fn run_generic<N: Network>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        access_key: Option<TempoAccountsWallet>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        self.run_generic_inner::<N>(pre_resolved_signer, access_key, None).await
    }

    async fn run_generic_inner<N: Network>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        mut access_key: Option<TempoAccountsWallet>,
        pre_resolved_browser: Option<BrowserSigner<N>>,
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
        tempo::ensure_session_not_browser(&tx.tempo, send_tx.browser.browser)?;

        let print_sponsor_hash = tx.tempo.print_sponsor_hash;
        let sponsor_url = tx.tempo.sponsor_url.clone();
        let sponsor_fee_payer = tx.tempo.sponsor;
        let expires_at = tx.tempo.resolve_expires();
        let tempo_sponsor = if print_sponsor_hash || sponsor_url.is_some() {
            None
        } else {
            tx.tempo.sponsor_config().await?
        };

        let blob_data = path.map(std::fs::read).transpose()?;

        if let Some(data) = data {
            sig = Some(data);
        }

        let code = if let Some(SendTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            // 7702 and 4844 transactions require a target, so they can't be CREATE transactions.
            if to.is_none() && !tx.auth.is_empty() {
                return Err(eyre!(
                    "EIP-7702 transactions can't be CREATE transactions and require a destination address"
                ));
            }
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
        if let Some(to_addr) = &to {
            let is_factory = match to_addr {
                NameOrAddress::Address(addr) => *addr == TIP20_FACTORY_ADDRESS,
                NameOrAddress::Name(name) => {
                    Address::from_str(name).ok() == Some(TIP20_FACTORY_ADDRESS)
                }
            };

            if !force
                && is_factory
                && let Some(sig_str) = &sig
                && sig_str.starts_with("createToken")
                && let Some(currency) = args.get(2)
                && !is_iso4217_currency(currency)
            {
                sh_warn!("{}", iso4217_warning_message(currency))?;
                if !confirm_continue()? {
                    return Ok(());
                }
            }
        }

        let config = send_tx.eth.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
        // The provider is not consulted for fee tokens in `--curl` mode.
        let fee_provider = (!config.eth_rpc_curl).then_some(&provider);

        let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;
        let lane = resolved_lane.as_ref();

        apply_poll_interval(&provider, send_tx.poll_interval);

        if has_session || access_key.is_some() {
            let chain = get_chain(config.chain, &provider).await?;
            access_key =
                tempo::resolve_session_or_wallet_signer(&tx.tempo, &send_tx.eth.wallet, chain.id())
                    .await?
                    .1;
        }

        // Inject access key ID into TempoOpts so it's set before gas estimation.
        if let Some(ak) = &access_key {
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
        let chain = builder.chain();

        // If --tempo.print-sponsor-hash was passed, build the tx, print the hash, and exit.
        if print_sponsor_hash {
            // The sponsor hash commits to the sender, so resolve the actual sender first.
            let (mut tx, from) = if let Some(ak) = &access_key {
                let Some((tx, prepared)) =
                    confirm_and_build_with_tempo_wallet(builder, ak, force, None).await?
                else {
                    return Ok(());
                };
                (tx, prepared.account())
            } else {
                let signer = pre_resolved_signer.as_ref().ok_or_else(|| {
                    eyre!("--tempo.print-sponsor-hash requires a signer (e.g. --private-key)")
                })?;
                let Some(tx) = confirm_and_build(builder, signer, force, None, false).await? else {
                    return Ok(());
                };
                (tx, signer.address())
            };
            let hash =
                tempo::sponsor_hash(fee_provider, chain, &mut tx, from, sponsor_fee_payer).await?;
            sh_println!("{hash:?}")?;
            return Ok(());
        }

        tempo::print_expires(expires_at)?;

        // Without a sponsor the fee token is resolved for the sender while sending.
        let send_opts = SendOptions::new(&send_tx, &config);
        let fee_send_opts =
            send_opts.resolving_fee_token(tempo_sponsor.is_none().then_some(chain), &config);

        // --sponsor-url is valid with local signers and Tempo access keys. Bail early rather than
        // silently ignoring it in signing paths that cannot produce a raw transaction locally.
        if let Some(url) = &sponsor_url {
            validate_sponsor_url(url)?;
            if unlocked {
                eyre::bail!("--sponsor-url cannot be combined with --unlocked");
            }
            if send_tx.browser.browser {
                eyre::bail!("--sponsor-url cannot be combined with --browser");
            }
        }

        // Launch a browser signer if one was not already resolved by the caller.
        let browser = if let Some(browser) = pre_resolved_browser {
            Some(browser)
        } else {
            send_tx.browser.run::<N>().await?
        };

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked && browser.is_none() {
            // Switch chain if the current chain id is not the one specified in the config.
            if let Some(config_chain) = config.chain {
                let config_chain_id = config_chain.id();
                if config_chain_id != provider.get_chain_id().await? {
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

            let Some(mut tx_request) =
                confirm_and_build(builder, config.sender, force, lane, false).await?
            else {
                return Ok(());
            };
            tempo::maybe_attach_sponsor(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx_request,
                config.sender,
            )
            .await?;

            cast_send(provider, tx_request, &fee_send_opts).await?;
        // Case 2:
        // Browser wallet signs and sends the transaction in one step.
        } else if let Some(browser) = browser {
            let from = browser.address();
            let Some(mut tx_request) =
                confirm_and_build(builder.with_browser_wallet(), from, force, lane, false).await?
            else {
                return Ok(());
            };
            tempo::apply_fee_payment::<N, _>(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx_request,
                from,
            )
            .await?;

            if chain.id() != browser.chain_id() {
                sh_warn!("Switching browser wallet to chain {}", chain)?;
                browser.switch_chain(chain.id()).await?;
            }

            let tx_hash = browser.send_transaction_via_browser(tx_request).await?;
            send_opts.print_tx_result(&provider, tx_hash).await?;
        // Case 3: Tempo access-key wallet.
        } else if let Some(ak) = access_key {
            let Some((mut tx_request, prepared)) =
                confirm_and_build_with_tempo_wallet(builder, &ak, force, lane).await?
            else {
                return Ok(());
            };
            tempo::maybe_attach_sponsor(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx_request,
                prepared.account(),
            )
            .await?;
            if let Some(sponsor_url) = sponsor_url.as_deref() {
                cast_send_with_tempo_wallet_via_sponsor(
                    &provider,
                    tx_request,
                    &prepared,
                    sponsor_url,
                    &send_opts,
                )
                .await?;
            } else {
                cast_send_with_tempo_wallet(&provider, tx_request, &prepared, &fee_send_opts)
                    .await?;
            }
        // Case 4: a local signer.
        // If we cannot successfully instantiate a local signer, then we will assume we don't have
        // enough information to sign and we must bail.
        } else {
            let (signer, from) = tx::resolve_send_signer(pre_resolved_signer, &send_tx.eth).await?;
            let Some(mut tx_request) =
                confirm_and_build(builder, &signer, force, lane, false).await?
            else {
                return Ok(());
            };
            let wallet_provider = AlloyProviderBuilder::<_, _, N>::default();

            if let Some(sponsor_url) = sponsor_url {
                // Sign locally, ask the sponsor service for a fee-payer signature, then submit the
                // fully-sponsored tx to the regular RPC.
                tx_request.set_fee_payer_signature(FEE_PAYER_SIGNATURE_MARKER);
                let connector = tempo::sponsor_relay_connector(&provider, &sponsor_url)?;
                let provider = wallet_provider
                    .wallet(EthereumWallet::from(signer))
                    .connect_with(&connector)
                    .await?;
                cast_send(provider, tx_request, &send_opts).await?;
            } else {
                tempo::maybe_attach_sponsor(
                    tempo_sponsor.as_ref(),
                    fee_provider,
                    chain,
                    &mut tx_request,
                    from,
                )
                .await?;
                let provider = wallet_provider
                    .wallet(EthereumWallet::from(signer))
                    .connect_provider(&provider);
                cast_send(provider, tx_request, &fee_send_opts).await?;
            }
        }

        Ok(())
    }
}

/// How a transaction is submitted and its result reported.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SendOptions {
    /// Only print the transaction hash instead of waiting for the receipt.
    cast_async: bool,
    /// Submit with the synchronous RPC methods and print the returned receipt.
    sync: bool,
    confirmations: u64,
    timeout: u64,
    /// Chain used to resolve a missing Tempo fee token for the sender before sending. `None`
    /// leaves the fee token as built, e.g. when a sponsor already selected it.
    fee_chain: Option<Chain>,
    /// Whether the provider may be queried for the stored fee token and its symbol.
    query_fee_token: bool,
}

impl SendOptions {
    /// Submission options from the CLI flags and config, without fee token resolution.
    pub(crate) fn new(send_tx: &SendTxOpts, config: &Config) -> Self {
        Self {
            cast_async: send_tx.cast_async,
            sync: send_tx.sync,
            confirmations: send_tx.confirmations,
            timeout: send_tx.timeout.unwrap_or(config.transaction_timeout),
            fee_chain: None,
            query_fee_token: false,
        }
    }

    /// Resolves the sender's fee token on `chain` before sending, querying the RPC unless the
    /// request is only rendered as `curl`.
    pub(crate) const fn resolving_fee_token(self, chain: Option<Chain>, config: &Config) -> Self {
        Self { fee_chain: chain, query_fee_token: chain.is_some() && !config.eth_rpc_curl, ..self }
    }

    /// Prints the hash of a submitted transaction, or its receipt unless `--async` was passed.
    pub(crate) async fn print_tx_result<N: Network, P: Provider<N>>(
        &self,
        provider: P,
        tx_hash: B256,
    ) -> Result<()>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        CastTxSender::new(provider)
            .print_tx_result(tx_hash, self.cast_async, self.confirmations, self.timeout)
            .await
    }

    /// Prints a sync receipt, or the hash / polled receipt of a pending transaction.
    async fn print_send_result<N: Network, P: Provider<N>>(
        &self,
        provider: P,
        tx_hash: B256,
        receipt: Option<String>,
    ) -> Result<B256>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        match receipt {
            Some(receipt) => sh_println!("{receipt}")?,
            None => self.print_tx_result(provider, tx_hash).await?,
        }
        Ok(tx_hash)
    }

    async fn resolve_and_print_fee_token<N: Network, P: Provider<N>>(
        &self,
        provider: &P,
        tx: &mut N::TransactionRequest,
    ) -> Result<()>
    where
        N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    {
        tempo::resolve_and_print_fee_token(
            self.query_fee_token.then_some(provider),
            self.fee_chain,
            tx,
            None,
        )
        .await
    }
}

/// Sends a transaction through `provider`, which signs it (a wallet-filled provider or an
/// unlocked RPC account).
pub(crate) async fn cast_send<N: Network, P: Provider<N>>(
    provider: P,
    mut tx: N::TransactionRequest,
    opts: &SendOptions,
) -> Result<B256>
where
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    opts.resolve_and_print_fee_token(&provider, &mut tx).await?;
    let (tx_hash, receipt) = if opts.sync {
        // JSON envelope not supported: N::ReceiptResponse is generic over Display but not
        // Serialize; adding Serialize would ripple across all network-generic callers.
        let (tx_hash, receipt) = CastTxSender::new(&provider).send_sync(tx).await?;
        (tx_hash, Some(receipt))
    } else {
        (*CastTxSender::new(&provider).send(tx).await?.tx_hash(), None)
    };
    opts.print_send_result(provider, tx_hash, receipt).await
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
        Ok((*provider.send_raw_transaction(raw_tx).await?.tx_hash(), None))
    }
}

/// Signs a prepared transaction with a Tempo wallet and sends it as a raw transaction.
pub(crate) async fn cast_send_with_tempo_wallet<N: Network, P: Provider<N>>(
    provider: &P,
    mut tx: N::TransactionRequest,
    wallet: &TempoAccountsWallet,
    opts: &SendOptions,
) -> Result<B256>
where
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    opts.resolve_and_print_fee_token(provider, &mut tx).await?;
    let raw_tx = tx.sign_with_tempo_wallet(wallet).await?;
    let (tx_hash, receipt) = cast_send_raw(provider, &raw_tx, opts.sync).await?;
    opts.print_send_result(provider, tx_hash, receipt).await
}

/// Signs a prepared transaction with a Tempo wallet, obtains a remote fee-payer signature, and
/// broadcasts the sponsored transaction through the original provider transport. The relay
/// selects the fee token, so `opts` must not resolve one.
pub(crate) async fn cast_send_with_tempo_wallet_via_sponsor<N: Network, P: Provider<N>>(
    provider: &P,
    mut tx: N::TransactionRequest,
    wallet: &TempoAccountsWallet,
    sponsor_url: &str,
    opts: &SendOptions,
) -> Result<B256>
where
    N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    tx.set_fee_payer_signature(FEE_PAYER_SIGNATURE_MARKER);
    let connector = tempo::sponsor_relay_connector(provider, sponsor_url)?;
    let sponsor_provider =
        AlloyProviderBuilder::<_, _, N>::default().connect_with(&connector).await?;
    cast_send_with_tempo_wallet(&sponsor_provider, tx, wallet, opts).await
}

/// Validates that a sponsor URL uses https:// (localhost/127.0.0.1 may use http://).
pub(crate) fn validate_sponsor_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw)
        .map_err(|e| eyre::eyre!("--sponsor-url is not a valid URL ({raw}): {e}"))?;
    match url.scheme() {
        "https" => Ok(()),
        "http" if matches!(url.host_str(), Some("localhost" | "127.0.0.1")) => Ok(()),
        "http" => eyre::bail!(
            "--sponsor-url must use https:// for non-local endpoints (got {raw}). \
             The sponsor relay is a trusted third party; use an encrypted channel."
        ),
        _ => eyre::bail!(
            "--sponsor-url must start with https:// (got {raw}). \
             The sponsor relay is a trusted third party; use an encrypted channel."
        ),
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

        let opts = SendOptions {
            cast_async: false,
            sync: true,
            confirmations: 1,
            timeout: 1,
            fee_chain: None,
            query_fee_token: false,
        };
        let actual_hash = cast_send_with_tempo_wallet(&provider, tx, &wallet, &opts).await.unwrap();

        assert_eq!(actual_hash, tx_hash);
        assert_eq!(methods.lock().unwrap().as_slice(), ["eth_sendRawTransactionSync"]);
    }
}
