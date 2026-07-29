use std::{str::FromStr, time::Duration};

use crate::{
    SimpleCast,
    cmd::{
        call_overrides::CallOverrideOpts,
        send::{
            cast_send, cast_send_with_tempo_wallet, cast_send_with_tempo_wallet_via_sponsor,
            validate_sponsor_url,
        },
    },
    format_uint_exp, tempo,
    tx::{CastTxSender, SendTxOpts, TxParams, fill_transaction_gas_fees},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network, TransactionBuilder};
use alloy_primitives::{Address, U256};
use alloy_provider::{
    Provider, ProviderBuilder as AlloyProviderBuilder, fillers::RecommendedFillers,
};
use alloy_signer::{Signature, Signer};
use alloy_sol_types::sol;
use clap::{Args, Parser};
use eyre::WrapErr;
use foundry_cli::{
    json::{print_json_success, print_scalar},
    opts::RpcOpts,
    utils::{LoadConfig, get_chain, get_provider},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::{ProviderBuilder, RetryProviderWithSigner},
    shell,
    tempo::{TEMPO_BROWSER_GAS_BUFFER, maybe_print_fee_token, resolve_and_set_fee_token},
};
#[doc(hidden)]
pub use foundry_config::{Chain, Eip1559FeeEstimatePreset, utils::*};
use foundry_wallets::{TempoAccountsWallet, WalletSigner};
use tempo_alloy::TempoNetwork;
use tempo_primitives::transaction::FEE_PAYER_SIGNATURE_MARKER;

sol! {
    #[sol(rpc)]
    interface IERC20 {
        #[derive(Debug)]
        function name() external view returns (string);
        function symbol() external view returns (string);
        function decimals() external view returns (uint256);
        function totalSupply() external view returns (uint256);
        function balanceOf(address owner) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function approve(address spender, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function mint(address to, uint256 amount) external;
        function burn(uint256 amount) external;
    }
}

const AUTO_UNITS_ERROR: &str = "failed to query ERC-20 `decimals()`; use an explicit decimal \
                                count with `--units` for tokens with missing or nonstandard \
                                metadata";

/// How ERC-20 amounts are interpreted.
#[derive(Clone, Copy, Debug, Default)]
enum Erc20Units {
    /// Base-unit integers.
    #[default]
    Raw,
    /// The value returned by the token's `decimals()` function.
    Auto,
    /// An explicit decimal count.
    Decimals(u8),
}

impl FromStr for Erc20Units {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "raw" => Ok(Self::Raw),
            "auto" => Ok(Self::Auto),
            _ => value.parse().map(Self::Decimals).map_err(|_| {
                format!("invalid units `{value}`: expected `raw`, `auto`, or a decimal count")
            }),
        }
    }
}

/// ERC-20 amount unit options.
#[derive(Args, Clone, Copy, Debug, Default)]
pub struct Erc20UnitsOpts {
    /// Interpret amounts as raw base units, with a decimal count, or by querying `decimals()`.
    ///
    /// `auto` accepts an ABI integer from 0 to 255, including an ABI-compatible `uint256`,
    /// and fails if `decimals()` is missing, reverts, or returns invalid data. Pass a decimal
    /// count to bypass token metadata.
    #[arg(long, default_value = "raw", value_name = "DECIMALS|auto|raw")]
    units: Erc20Units,
}

macro_rules! resolve_erc20_decimals {
    ($erc20:expr, $units:expr, $block:expr) => {{
        match $units.units {
            Erc20Units::Raw => None,
            Erc20Units::Decimals(decimals) => Some(decimals),
            Erc20Units::Auto => {
                let decimals =
                    $erc20.decimals().block($block).call().await.wrap_err(AUTO_UNITS_ERROR)?;
                Some(parse_erc20_decimals(decimals).wrap_err(AUTO_UNITS_ERROR)?)
            }
        }
    }};
}

fn parse_erc20_decimals(decimals: U256) -> eyre::Result<u8> {
    eyre::ensure!(
        decimals <= U256::from(u8::MAX),
        "expected a value between 0 and 255, got {decimals}"
    );
    Ok(decimals.to::<u8>())
}

fn validate_erc20_amount_precision(amount: &str, decimals: u8) -> eyre::Result<()> {
    if let Some((_, fractional)) = amount.split_once('.') {
        let precision = usize::from(decimals);
        let fractional = fractional.as_bytes();
        eyre::ensure!(
            fractional.len() <= precision
                || fractional[precision..].iter().all(|&digit| digit == b'0'),
            "amount has more than {decimals} decimal places"
        );
    }
    Ok(())
}

fn parse_erc20_amount(amount: &str, decimals: Option<u8>) -> eyre::Result<U256> {
    let Some(decimals) = decimals else {
        return Ok(U256::from_str(amount)?);
    };

    validate_erc20_amount_precision(amount, decimals).wrap_err_with(|| {
        format!("failed to parse ERC-20 amount `{amount}` with {decimals} decimals")
    })?;
    let amount = SimpleCast::parse_units(amount, decimals).wrap_err_with(|| {
        format!("failed to parse ERC-20 amount `{amount}` with {decimals} decimals")
    })?;
    Ok(U256::from_str(&amount)?)
}

fn format_erc20_amount(amount: U256, decimals: u8) -> eyre::Result<String> {
    SimpleCast::format_units(&amount.to_string(), decimals)
        .wrap_err_with(|| format!("failed to format ERC-20 amount with {decimals} decimals"))
}

/// Creates a provider with a pre-resolved signer.
pub(crate) fn build_provider_with_signer<N: Network + RecommendedFillers>(
    tx_opts: &SendTxOpts,
    signer: WalletSigner,
) -> eyre::Result<RetryProviderWithSigner<N>>
where
    N::TxEnvelope: From<Signed<N::UnsignedTx>>,
    N::UnsignedTx: SignableTransaction<Signature>,
{
    let config = tx_opts.eth.load_config()?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::<N>::from_config(&config)?.build_with_wallet(wallet)?;
    if let Some(interval) = tx_opts.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval))
    }
    Ok(provider)
}

/// Interact with ERC20 tokens.
#[derive(Debug, Parser, Clone)]
pub enum Erc20Subcommand {
    /// Query ERC20 token balance.
    #[command(visible_alias = "b")]
    Balance {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The owner to query balance for.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        overrides: CallOverrideOpts,

        #[command(flatten)]
        units: Erc20UnitsOpts,
    },

    /// Transfer ERC20 tokens.
    #[command(visible_aliases = ["t", "send"])]
    Transfer {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The recipient address.
        #[arg(value_parser = NameOrAddress::from_str)]
        to: NameOrAddress,

        /// The amount to transfer.
        amount: String,

        #[command(flatten)]
        units: Erc20UnitsOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Approve ERC20 token spending.
    #[command(visible_alias = "a")]
    Approve {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The spender address.
        #[arg(value_parser = NameOrAddress::from_str)]
        spender: NameOrAddress,

        /// The amount to approve.
        amount: String,

        #[command(flatten)]
        units: Erc20UnitsOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Query ERC20 token allowance.
    #[command(visible_alias = "al")]
    Allowance {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The owner address.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        /// The spender address.
        #[arg(value_parser = NameOrAddress::from_str)]
        spender: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        units: Erc20UnitsOpts,
    },

    /// Query ERC20 token name.
    #[command(visible_alias = "n")]
    Name {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query ERC20 token symbol.
    #[command(visible_alias = "s")]
    Symbol {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query ERC20 token decimals.
    #[command(visible_alias = "d")]
    Decimals {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query ERC20 token total supply.
    #[command(visible_alias = "ts")]
    TotalSupply {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        units: Erc20UnitsOpts,
    },

    /// Mint ERC20 tokens (if the token supports minting).
    #[command(visible_alias = "m")]
    Mint {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The recipient address.
        #[arg(value_parser = NameOrAddress::from_str)]
        to: NameOrAddress,

        /// The amount to mint.
        amount: String,

        #[command(flatten)]
        units: Erc20UnitsOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Burn ERC20 tokens.
    #[command(visible_alias = "bu")]
    Burn {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The amount to burn.
        amount: String,

        #[command(flatten)]
        units: Erc20UnitsOpts,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },
}

impl Erc20Subcommand {
    const fn rpc_opts(&self) -> &RpcOpts {
        match self {
            Self::Allowance { rpc, .. } => rpc,
            Self::Approve { send_tx, .. } => &send_tx.eth.rpc,
            Self::Balance { rpc, .. } => rpc,
            Self::Transfer { send_tx, .. } => &send_tx.eth.rpc,
            Self::Name { rpc, .. } => rpc,
            Self::Symbol { rpc, .. } => rpc,
            Self::Decimals { rpc, .. } => rpc,
            Self::TotalSupply { rpc, .. } => rpc,
            Self::Mint { send_tx, .. } => &send_tx.eth.rpc,
            Self::Burn { send_tx, .. } => &send_tx.eth.rpc,
        }
    }

    const fn erc20_opts(&self) -> Option<&TxParams> {
        match self {
            Self::Approve { tx, .. }
            | Self::Transfer { tx, .. }
            | Self::Mint { tx, .. }
            | Self::Burn { tx, .. } => Some(tx),
            Self::Allowance { .. }
            | Self::Balance { .. }
            | Self::Name { .. }
            | Self::Symbol { .. }
            | Self::Decimals { .. }
            | Self::TotalSupply { .. } => None,
        }
    }

    const fn uses_browser_send(&self) -> bool {
        match self {
            Self::Transfer { send_tx, .. }
            | Self::Approve { send_tx, .. }
            | Self::Mint { send_tx, .. }
            | Self::Burn { send_tx, .. } => send_tx.browser.browser,
            _ => false,
        }
    }

    async fn should_use_tempo_network(
        &self,
        tempo_access_key: &Option<TempoAccountsWallet>,
        has_session: bool,
    ) -> eyre::Result<bool> {
        if self.erc20_opts().is_some_and(|erc20| erc20.tempo.is_tempo())
            || has_session
            || tempo_access_key.is_some()
        {
            return Ok(true);
        }

        if self.uses_browser_send() {
            let config = self.rpc_opts().load_config()?;
            return Ok(get_chain(config.chain, &get_provider(&config)?).await?.is_tempo());
        }

        Ok(false)
    }

    fn has_tempo_session(&self) -> eyre::Result<bool> {
        self.erc20_opts().map_or(Ok(false), |opts| opts.tempo.session_id().map(|id| id.is_some()))
    }

    pub async fn run(self) -> eyre::Result<()> {
        let has_session = self.has_tempo_session()?;
        // Resolve the signer once for state-changing variants.
        let (resolved_tempo, signer, tempo_access_key) = match &self {
            Self::Transfer { send_tx, tx, .. }
            | Self::Approve { send_tx, tx, .. }
            | Self::Mint { send_tx, tx, .. }
            | Self::Burn { send_tx, tx, .. } => {
                // Explicit Tempo sessions are resolved after network selection, once the chain is
                // known.
                if has_session {
                    (true, None, None)
                } else {
                    tempo::resolve_transaction_network_and_signer(&tx.tempo, &send_tx.eth).await?
                }
            }
            _ => (false, None, None),
        };

        let is_tempo =
            resolved_tempo || self.should_use_tempo_network(&tempo_access_key, has_session).await?;

        if is_tempo {
            self.run_generic::<TempoNetwork>(signer, tempo_access_key, has_session).await
        } else {
            self.run_generic::<Ethereum>(signer, None, has_session).await
        }
    }

    #[allow(clippy::large_stack_frames)]
    pub async fn run_generic<N: Network + RecommendedFillers>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        tempo_keychain: Option<TempoAccountsWallet>,
        has_session: bool,
    ) -> eyre::Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let config = self.rpc_opts().load_config()?;

        // Macro to DRY the keychain-vs-normal send pattern for state-changing ops.
        // The only thing that varies per variant is the IERC20 call expression.
        macro_rules! erc20_send {
            (
                $token:expr,
                $send_tx:expr,
                $tx_opts:expr, |
                $erc20:ident,
                $provider:ident |
                $build_tx:expr
            ) => {{
                let mut tx_opts = $tx_opts;
                tempo::ensure_session_not_browser(&tx_opts.tempo, $send_tx.browser.browser)?;
                let (pre_resolved_signer, tempo_keychain) =
                    if has_session || tempo_keychain.is_some() {
                        let $provider =
                            ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
                        let chain = get_chain(config.chain, &$provider).await?;
                        tempo::resolve_session_or_wallet_signer(
                            &tx_opts.tempo,
                            &$send_tx.eth.wallet,
                            chain.id(),
                        )
                        .await?
                    } else {
                        (pre_resolved_signer, tempo_keychain)
                    };
                let print_sponsor_hash = tx_opts.tempo.print_sponsor_hash;
                let sponsor_url = tx_opts.tempo.sponsor_url.clone();
                let sponsor_fee_payer = tx_opts.tempo.sponsor;
                let expires_at = tx_opts.tempo.resolve_expires();
                let tempo_sponsor = if print_sponsor_hash || sponsor_url.is_some() {
                    None
                } else {
                    tx_opts.tempo.sponsor_config().await?
                };
                let needs_sponsor_payload =
                    print_sponsor_hash || tempo_sponsor.is_some() || sponsor_url.is_some();
                if let Some(ref url) = sponsor_url {
                    validate_sponsor_url(url)?;
                    if $send_tx.browser.browser {
                        eyre::bail!("--sponsor-url cannot be combined with --browser");
                    }
                }
                if let Some(ts) = expires_at {
                    sh_status!("Transaction expires at unix timestamp {ts}")?;
                }

                let timeout = $send_tx.timeout.unwrap_or(config.transaction_timeout);
                if let Some(ref access_key) = tempo_keychain {
                    let $provider =
                        ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
                    let $erc20 = IERC20::new($token.resolve(&$provider).await?, &$provider);
                    let mut tx = { $build_tx }.into_transaction_request();
                    let chain = get_chain(config.chain, &$provider).await?;
                    tx_opts.apply::<TempoNetwork>(&mut tx, chain.is_legacy());
                    let prepared_access_key = tempo::fill_access_key_transaction(
                        &$provider,
                        &mut tx,
                        access_key,
                        chain,
                        config.eip1559_fee_estimate,
                    )
                    .await?;
                    if needs_sponsor_payload {
                        if print_sponsor_hash {
                            if let Some(fee_payer) = sponsor_fee_payer {
                                resolve_and_set_fee_token(
                                    (!config.eth_rpc_curl).then_some(&$provider),
                                    Some(chain),
                                    &mut tx,
                                    Some(fee_payer),
                                )
                                .await?;
                            }
                            let hash = tx
                                .compute_sponsor_hash(prepared_access_key.account())
                                .ok_or_else(|| {
                                    eyre::eyre!(
                                        "This network does not support sponsored transactions"
                                    )
                                })?;
                            sh_println!("{hash:?}")?;
                            return Ok(());
                        }
                        if let Some(sponsor) = &tempo_sponsor {
                            sponsor
                                .resolve_and_set_fee_token(
                                    (!config.eth_rpc_curl).then_some(&$provider),
                                    Some(chain),
                                    &mut tx,
                                )
                                .await?;
                            sponsor
                                .attach_and_print::<TempoNetwork>(
                                    &mut tx,
                                    prepared_access_key.account(),
                                )
                                .await?;
                        }
                    }
                    if let Some(sponsor_url) = sponsor_url.as_deref() {
                        cast_send_with_tempo_wallet_via_sponsor(
                            &$provider,
                            tx,
                            &prepared_access_key,
                            sponsor_url,
                            $send_tx.cast_async,
                            $send_tx.sync,
                            $send_tx.confirmations,
                            timeout,
                        )
                        .await?;
                    } else {
                        cast_send_with_tempo_wallet(
                            &$provider,
                            tx,
                            &prepared_access_key,
                            tempo_sponsor.is_none().then_some(chain),
                            None,
                            $send_tx.cast_async,
                            $send_tx.sync,
                            $send_tx.confirmations,
                            timeout,
                            tempo_sponsor.is_none() && !config.eth_rpc_curl,
                        )
                        .await?;
                    }
                } else if let Some(browser) = $send_tx.browser.run::<N>().await? {
                    let $provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
                    if let Some(interval) = $send_tx.poll_interval {
                        $provider.client().set_poll_interval(Duration::from_secs(interval));
                    }
                    let $erc20 = IERC20::new($token.resolve(&$provider).await?, &$provider);
                    let mut tx = { $build_tx }.into_transaction_request();
                    let chain = get_chain(config.chain, &$provider).await?;
                    tx_opts.apply::<N>(&mut tx, chain.is_legacy());
                    fill_tx(
                        &$provider,
                        &mut tx,
                        browser.address(),
                        chain,
                        true,
                        config.eip1559_fee_estimate,
                    )
                    .await?;
                    if print_sponsor_hash {
                        if let Some(fee_payer) = sponsor_fee_payer {
                            resolve_and_set_fee_token(
                                (!config.eth_rpc_curl).then_some(&$provider),
                                Some(chain),
                                &mut tx,
                                Some(fee_payer),
                            )
                            .await?;
                        }
                        let hash = tx.compute_sponsor_hash(browser.address()).ok_or_else(|| {
                            eyre::eyre!("This network does not support sponsored transactions")
                        })?;
                        sh_println!("{hash:?}")?;
                        return Ok(());
                    }
                    if let Some(sponsor) = &tempo_sponsor {
                        sponsor
                            .resolve_and_set_fee_token(
                                (!config.eth_rpc_curl).then_some(&$provider),
                                Some(chain),
                                &mut tx,
                            )
                            .await?;
                        sponsor.attach_and_print::<N>(&mut tx, browser.address()).await?;
                    } else {
                        let fee_token = resolve_and_set_fee_token(
                            (!config.eth_rpc_curl).then_some(&$provider),
                            Some(chain),
                            &mut tx,
                            Some(browser.address()),
                        )
                        .await?;
                        maybe_print_fee_token(
                            (!config.eth_rpc_curl).then_some(&$provider),
                            fee_token,
                        )
                        .await?;
                    }
                    let tx_hash = browser.send_transaction_via_browser(tx).await?;
                    CastTxSender::new(&$provider)
                        .print_tx_result(
                            tx_hash,
                            $send_tx.cast_async,
                            $send_tx.confirmations,
                            timeout,
                        )
                        .await?
                } else {
                    let signer = pre_resolved_signer.unwrap_or($send_tx.eth.wallet.signer().await?);
                    let from = signer.address();
                    let wallet = EthereumWallet::from(signer);
                    let $provider = ProviderBuilder::<N>::from_config(&config)?
                        .build_with_wallet(wallet.clone())?;
                    if let Some(interval) = $send_tx.poll_interval {
                        $provider.client().set_poll_interval(Duration::from_secs(interval));
                    }
                    let $erc20 = IERC20::new($token.resolve(&$provider).await?, &$provider);
                    let mut tx = { $build_tx }.into_transaction_request();
                    let chain = get_chain(config.chain, &$provider).await?;
                    tx_opts.apply::<N>(&mut tx, chain.is_legacy());
                    if needs_sponsor_payload {
                        fill_tx(
                            &$provider,
                            &mut tx,
                            from,
                            chain,
                            false,
                            config.eip1559_fee_estimate,
                        )
                        .await?;
                        if print_sponsor_hash {
                            if let Some(fee_payer) = sponsor_fee_payer {
                                resolve_and_set_fee_token(
                                    (!config.eth_rpc_curl).then_some(&$provider),
                                    Some(chain),
                                    &mut tx,
                                    Some(fee_payer),
                                )
                                .await?;
                            }
                            let hash = tx.compute_sponsor_hash(from).ok_or_else(|| {
                                eyre::eyre!("This network does not support sponsored transactions")
                            })?;
                            sh_println!("{hash:?}")?;
                            return Ok(());
                        }
                        if let Some(sponsor) = &tempo_sponsor {
                            sponsor
                                .resolve_and_set_fee_token(
                                    (!config.eth_rpc_curl).then_some(&$provider),
                                    Some(chain),
                                    &mut tx,
                                )
                                .await?;
                            sponsor.attach_and_print::<N>(&mut tx, from).await?;
                        }
                    } else {
                        // Fill only the fees; the provider fills nonce and gas limit.
                        fill_transaction_gas_fees(
                            &$provider,
                            &mut tx,
                            chain.is_legacy(),
                            false,
                            config.eip1559_fee_estimate,
                        )
                        .await?;
                    }
                    if let Some(sponsor_url) = sponsor_url {
                        tx.set_fee_payer_signature(FEE_PAYER_SIGNATURE_MARKER);
                        let connector = tempo::sponsor_relay_connector(&$provider, &sponsor_url)?;
                        let provider = AlloyProviderBuilder::<_, _, N>::default()
                            .wallet(wallet)
                            .connect_with(&connector)
                            .await?;
                        cast_send(
                            provider,
                            tx,
                            None,
                            None,
                            $send_tx.cast_async,
                            $send_tx.sync,
                            $send_tx.confirmations,
                            timeout,
                            false,
                        )
                        .await?;
                    } else {
                        cast_send(
                            $provider,
                            tx,
                            tempo_sponsor.is_none().then_some(chain),
                            None,
                            $send_tx.cast_async,
                            $send_tx.sync,
                            $send_tx.confirmations,
                            timeout,
                            tempo_sponsor.is_none() && !config.eth_rpc_curl,
                        )
                        .await?;
                    }
                }
            }};
        }

        match self {
            // Read-only
            Self::Allowance { token, owner, spender, block, units, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;
                let spender = spender.resolve(&provider).await?;
                let block = block.unwrap_or_default();
                let erc20 = IERC20::new(token, &provider);
                let decimals = resolve_erc20_decimals!(erc20, units, block);

                let allowance = erc20.allowance(owner, spender).block(block).call().await?;

                if let Some(decimals) = decimals {
                    let allowance = format_erc20_amount(allowance, decimals)?;
                    if shell::is_json() {
                        print_json_success(allowance)?;
                    } else {
                        sh_println!("{allowance}")?;
                    }
                } else if shell::is_json() {
                    print_json_success(allowance.to_string())?;
                } else {
                    sh_println!("{}", format_uint_exp(allowance))?;
                }
            }
            Self::Balance { token, owner, block, overrides, units, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;
                let block = block.unwrap_or_default();

                let erc20 = IERC20::new(token, &provider);
                let decimals = match units.units {
                    Erc20Units::Raw => None,
                    Erc20Units::Decimals(decimals) => Some(decimals),
                    Erc20Units::Auto => {
                        let decimals_call = erc20.decimals().block(block);
                        let decimals = overrides
                            .apply(decimals_call.call())?
                            .await
                            .wrap_err(AUTO_UNITS_ERROR)?;
                        Some(parse_erc20_decimals(decimals).wrap_err(AUTO_UNITS_ERROR)?)
                    }
                };
                let balance_call = erc20.balanceOf(owner).block(block);
                let call = balance_call.call();
                let balance = overrides.apply(call)?.await?;

                if let Some(decimals) = decimals {
                    let balance = format_erc20_amount(balance, decimals)?;
                    if shell::is_json() {
                        print_json_success(balance)?;
                    } else {
                        sh_println!("{balance}")?;
                    }
                } else if shell::is_json() {
                    print_json_success(balance.to_string())?;
                } else {
                    sh_println!("{balance}")?;
                }
            }
            Self::Name { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let name = IERC20::new(token, &provider)
                    .name()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;

                print_scalar(name)?;
            }
            Self::Symbol { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let symbol = IERC20::new(token, &provider)
                    .symbol()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;

                print_scalar(symbol)?;
            }
            Self::Decimals { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let decimals = IERC20::new(token, &provider)
                    .decimals()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                let decimals =
                    parse_erc20_decimals(decimals).wrap_err("invalid ERC-20 `decimals()` value")?;
                print_scalar(decimals)?;
            }
            Self::TotalSupply { token, block, units, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;
                let block = block.unwrap_or_default();
                let erc20 = IERC20::new(token, &provider);
                let decimals = resolve_erc20_decimals!(erc20, units, block);

                let total_supply = erc20.totalSupply().block(block).call().await?;

                if let Some(decimals) = decimals {
                    let total_supply = format_erc20_amount(total_supply, decimals)?;
                    if shell::is_json() {
                        print_json_success(total_supply)?;
                    } else {
                        sh_println!("{total_supply}")?;
                    }
                } else if shell::is_json() {
                    print_json_success(total_supply.to_string())?;
                } else {
                    sh_println!("{}", format_uint_exp(total_supply))?
                }
            }
            // State-changing
            Self::Transfer { token, to, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    let decimals = resolve_erc20_decimals!(erc20, units, BlockId::default());
                    erc20.transfer(
                        to.resolve(&provider).await?,
                        parse_erc20_amount(&amount, decimals)?,
                    )
                })
            }
            Self::Approve { token, spender, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    let decimals = resolve_erc20_decimals!(erc20, units, BlockId::default());
                    erc20.approve(
                        spender.resolve(&provider).await?,
                        parse_erc20_amount(&amount, decimals)?,
                    )
                })
            }
            Self::Mint { token, to, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    let decimals = resolve_erc20_decimals!(erc20, units, BlockId::default());
                    erc20.mint(to.resolve(&provider).await?, parse_erc20_amount(&amount, decimals)?)
                })
            }
            Self::Burn { token, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    let decimals = resolve_erc20_decimals!(erc20, units, BlockId::default());
                    erc20.burn(parse_erc20_amount(&amount, decimals)?)
                })
            }
        };
        Ok(())
    }
}

/// Fills from, chain_id, nonce, fees, and gas limit on a transaction request for sponsor/browser
/// wallet flows. Mirrors the filling logic in the shared tx builder but operates on a
/// pre-built transaction request from the sol! macro rather than through the builder pipeline.
/// Only fills fields that haven't already been set by the user.
async fn fill_tx<N: Network, P: Provider<N>>(
    provider: &P,
    tx: &mut N::TransactionRequest,
    from: Address,
    chain: Chain,
    browser: bool,
    eip1559_fee_estimate: Eip1559FeeEstimatePreset,
) -> eyre::Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    tx.set_from(from);
    tx.set_chain_id(chain.id());

    if tx.nonce().is_none() {
        tx.set_nonce(provider.get_transaction_count(from).await?);
    }

    let legacy = chain.is_legacy();

    fill_transaction_gas_fees(provider, tx, legacy, browser, eip1559_fee_estimate).await?;

    if tx.gas_limit().is_none() {
        let mut estimated = provider.estimate_gas(tx.clone()).await?;

        // Browser wallets may sign with P256/WebAuthn instead of secp256k1, which
        // costs more gas for signature verification on Tempo chains. Add a
        // conservative buffer since we can't determine the signature type beforehand.
        if chain.is_tempo() {
            estimated += TEMPO_BROWSER_GAS_BUFFER;
        }

        tx.set_gas_limit(estimated);
    }

    Ok(())
}
