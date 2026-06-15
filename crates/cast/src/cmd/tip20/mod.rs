use crate::{
    cmd::send::{cast_send, cast_send_with_access_key, validate_sponsor_url},
    tempo,
    tx::{CastTxBuilder, CastTxSender, SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, B256};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_rpc_client::BuiltInConnectionString;
use alloy_signer::Signer;
use clap::Parser;
use foundry_cli::{
    opts::TransactionOpts,
    utils::{LoadConfig, get_chain, maybe_print_resolved_lane, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    tempo::{TEMPO_BROWSER_GAS_BUFFER, maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use std::{str::FromStr, time::Duration};
use tempo_alloy::{
    TempoNetwork,
    transport::{RelayConnector, SponsorshipMode},
};
use tempo_primitives::transaction::FEE_PAYER_SIGNATURE_MARKER;

mod create;
pub(crate) use create::iso4217_warning_message;
pub(crate) mod logo;
pub(crate) mod mine;

/// TIP-20 token operations (Tempo).
#[derive(Debug, Parser, Clone)]
pub enum Tip20Subcommand {
    /// Create a new TIP-20 token via the TIP20Factory.
    #[command(visible_alias = "c")]
    Create {
        /// The token name (e.g. "US Dollar Coin").
        name: String,

        /// The token symbol (e.g. "USDC").
        symbol: String,

        /// The ISO 4217 currency code (e.g. "USD", "EUR", "GBP").
        /// This field is IMMUTABLE after creation and affects fee payment
        /// eligibility, DEX routing, and quote token pairing.
        currency: String,

        /// The TIP-20 quote token address used for exchange pricing.
        #[arg(value_parser = NameOrAddress::from_str)]
        quote_token: NameOrAddress,

        /// The admin address to receive DEFAULT_ADMIN_ROLE on the new token.
        #[arg(value_parser = NameOrAddress::from_str)]
        admin: NameOrAddress,

        /// A unique salt for deterministic address derivation (hex-encoded bytes32).
        salt: B256,

        /// Optional T5 logo URI for the token.
        #[arg(long, value_name = "URI")]
        logo_uri: Option<String>,

        /// Skip the ISO 4217 currency code validation warning.
        #[arg(long)]
        force: bool,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Validate a TIP-20 logo URI offline against Tempo T5 constraints.
    LogoCheck {
        /// The logo URI to validate. Empty string is valid.
        #[arg(value_name = "URI")]
        logo_uri: String,
    },

    /// Update a TIP-20 token logo URI.
    LogoSet {
        /// The TIP-20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The new logo URI. Empty string clears the on-chain value.
        #[arg(value_name = "URI")]
        logo_uri: String,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Mine a TIP-1022 salt for virtual address' master registration on Tempo.
    #[command(visible_alias = "m")]
    Mine {
        /// Address that will call `registerVirtualMaster(bytes32)`.
        #[arg(value_name = "ADDRESS")]
        master: Address,

        /// Salt to validate directly instead of mining one.
        #[arg(long, conflicts_with_all = ["seed", "no_random"], value_name = "HEX")]
        salt: Option<B256>,

        /// Number of threads to use. Specifying 0 defaults to the number of logical cores.
        #[arg(global = true, long, short = 'j', visible_alias = "jobs")]
        threads: Option<usize>,

        /// The random number generator's seed, used to initialize the salt search.
        #[arg(long, value_name = "HEX")]
        seed: Option<B256>,

        /// Don't initialize the salt with a random value, and instead use the default value of 0.
        #[arg(long, conflicts_with = "seed")]
        no_random: bool,

        /// Submit `registerVirtualMaster(bytes32)` on Tempo after finding or validating the salt.
        #[arg(long, conflicts_with_all = ["seed", "no_random"])]
        register: bool,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },
}

impl Tip20Subcommand {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            Self::Create {
                name,
                symbol,
                currency,
                quote_token,
                admin,
                salt,
                logo_uri,
                force,
                send_tx,
                tx,
            } => {
                create::run(
                    name,
                    symbol,
                    currency,
                    quote_token,
                    admin,
                    salt,
                    logo_uri,
                    force,
                    send_tx,
                    tx,
                )
                .await?;
            }
            Self::LogoCheck { logo_uri } => {
                logo::check(logo_uri)?;
            }
            Self::LogoSet { token, logo_uri, send_tx, tx } => {
                logo::set(token, logo_uri, send_tx, tx).await?;
            }
            Self::Mine { master, salt, threads, seed, no_random, register, send_tx, tx } => {
                let output = mine::run(master, salt, threads, seed, no_random)?;
                if register {
                    mine::register(master, output.salt, send_tx, tx).await?;
                }
            }
        }
        Ok(())
    }
}

pub(super) async fn resolve_tip20_signer(
    send_tx: &SendTxOpts,
    tx_params: &TxParams,
) -> eyre::Result<(Option<WalletSigner>, Option<TempoAccessKeyConfig>)> {
    if tx_params.tempo.session_id()?.is_none() {
        return send_tx.eth.wallet.maybe_signer().await;
    }

    tempo::ensure_session_not_browser(&tx_params.tempo, send_tx.browser.browser)?;

    let config = send_tx.eth.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    let chain = get_chain(config.chain, &provider).await?;
    tempo::resolve_session_or_wallet_signer(&tx_params.tempo, &send_tx.eth.wallet, chain.id()).await
}

pub(super) async fn send_tip20_transaction(
    to: NameOrAddress,
    sig: &'static str,
    args: Vec<String>,
    send_tx: SendTxOpts,
    tx_params: TxParams,
    pre_resolved_signer: Option<WalletSigner>,
    access_key: Option<TempoAccessKeyConfig>,
) -> eyre::Result<()> {
    let mut tx_opts = tx_params.into_transaction_opts();
    let print_sponsor_hash = tx_opts.tempo.print_sponsor_hash;
    let sponsor_url = tx_opts.tempo.sponsor_url.clone();
    let expires_at = tx_opts.tempo.resolve_expires();
    let tempo_sponsor = if print_sponsor_hash || sponsor_url.is_some() {
        None
    } else {
        tx_opts.tempo.sponsor_config().await?
    };

    if let Some(ref url) = sponsor_url {
        validate_sponsor_url(url)?;
        if send_tx.browser.browser {
            eyre::bail!("--sponsor-url cannot be combined with --browser");
        }
        if access_key.is_some() {
            eyre::bail!("--sponsor-url cannot be combined with a Tempo access key");
        }
    }

    let config = send_tx.eth.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    if let Some(interval) = send_tx.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval))
    }

    let resolved_lane = resolve_lane(&mut tx_opts.tempo, &config.root)?;
    if let Some(ref ak) = access_key {
        tx_opts.tempo.key_id = Some(ak.key_address);
    }

    let builder = CastTxBuilder::new(&provider, tx_opts, &config)
        .await?
        .with_to(Some(to))
        .await?
        .with_code_sig_and_args(None, Some(sig.to_string()), args)
        .await?;
    let chain = builder.chain();

    if print_sponsor_hash {
        let (tx, from) = if let Some(ref ak) = access_key {
            let (tx, _) = builder.build_with_access_key(ak.wallet_address, ak).await?;
            (tx, ak.wallet_address)
        } else {
            let signer = pre_resolved_signer.as_ref().ok_or_else(|| {
                eyre::eyre!("--tempo.print-sponsor-hash requires a signer (e.g. --private-key)")
            })?;
            let from = signer.address();
            let (tx, _) = builder.build(signer).await?;
            (tx, from)
        };
        let hash = tx
            .compute_sponsor_hash(from)
            .ok_or_else(|| eyre::eyre!("This network does not support sponsored transactions"))?;
        sh_println!("{hash:?}")?;
        return Ok(());
    }

    if let Some(ts) = expires_at {
        sh_status!("Transaction expires at unix timestamp {ts}")?;
    }

    let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
    if let Some(browser) = send_tx.browser.run::<TempoNetwork>().await? {
        let (mut tx, _) = builder.with_browser_wallet().build(browser.address()).await?;
        maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
        if let Some(gas) = tx.gas_limit() {
            tx.set_gas_limit(gas + TEMPO_BROWSER_GAS_BUFFER);
        }
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut tx, browser.address()).await?;
        } else {
            resolve_and_set_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                &mut tx,
                Some(browser.address()),
            )
            .await?;
            maybe_print_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                Some(&tx),
                None,
            )
            .await?;
        }
        let tx_hash = browser.send_transaction_via_browser(tx).await?;
        CastTxSender::new(&provider)
            .print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
            .await?;
    } else if let Some(ak) = access_key {
        let signer = pre_resolved_signer
            .as_ref()
            .ok_or_else(|| eyre::eyre!("signer required for access key"))?;
        let (mut tx, _) = builder.build_with_access_key(ak.wallet_address, &ak).await?;
        maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut tx, ak.wallet_address).await?;
        }
        cast_send_with_access_key(
            &provider,
            tx,
            signer,
            &ak,
            tempo_sponsor.is_none().then_some(chain),
            None,
            send_tx.cast_async,
            send_tx.confirmations,
            timeout,
            tempo_sponsor.is_none() && !config.eth_rpc_curl,
        )
        .await?;
    } else if let Some(sponsor_url) = sponsor_url {
        let signer = match pre_resolved_signer {
            Some(signer) => signer,
            None => send_tx.eth.wallet.signer().await?,
        };
        let from = signer.address();
        crate::tx::validate_from_address(send_tx.eth.wallet.from, from)?;

        let (mut tx, _) = builder.build(&signer).await?;
        maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
        tx.set_fee_payer_signature(FEE_PAYER_SIGNATURE_MARKER);

        let wallet = EthereumWallet::from(signer);
        let default_rpc = config.get_rpc_url_or_localhost_http()?.into_owned();
        let default = BuiltInConnectionString::from_str(&default_rpc)?;
        let relay = BuiltInConnectionString::from_str(&sponsor_url)?;
        let connector =
            RelayConnector::with_config(default, relay, SponsorshipMode::SignOnly, false);
        let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
            .wallet(wallet)
            .connect_with(&connector)
            .await?;
        cast_send(
            provider,
            tx,
            None,
            None,
            send_tx.cast_async,
            send_tx.sync,
            send_tx.confirmations,
            timeout,
            false,
        )
        .await?;
    } else {
        let signer = match pre_resolved_signer {
            Some(signer) => signer,
            None => send_tx.eth.wallet.signer().await?,
        };
        let from = signer.address();
        crate::tx::validate_from_address(send_tx.eth.wallet.from, from)?;

        let (mut tx, _) = builder.build(&signer).await?;
        maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<TempoNetwork>(&mut tx, from).await?;
        }

        let wallet = EthereumWallet::from(signer);
        let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
            .wallet(wallet)
            .connect_provider(&provider);
        cast_send(
            provider,
            tx,
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

impl TxParams {
    fn into_transaction_opts(self) -> TransactionOpts {
        TransactionOpts {
            gas_limit: self.gas_limit,
            gas_price: self.gas_price,
            priority_gas_price: self.priority_gas_price,
            value: None,
            nonce: self.nonce,
            legacy: false,
            blob: false,
            eip4844: false,
            blob_gas_price: None,
            auth: Vec::new(),
            access_list: None,
            tempo: self.tempo,
        }
    }
}
