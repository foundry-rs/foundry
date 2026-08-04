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
use alloy_primitives::{Address, U256, utils::Unit};
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
    tempo::{maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_wallets::{TempoAccountsWallet, WalletSigner};
use tempo_alloy::TempoNetwork;
use tempo_primitives::transaction::FEE_PAYER_SIGNATURE_MARKER;

#[doc(hidden)]
pub use foundry_config::{Chain, Eip1559FeeEstimatePreset, utils::*};

sol! {
    #[sol(rpc)]
    interface IERC20 {
        #[derive(Debug)]
        function name() external view returns (string);
        function symbol() external view returns (string);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address owner) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function approve(address spender, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function mint(address to, uint256 amount) external;
        function burn(uint256 amount) external;
    }
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

/// Controls how an ERC-20 amount is interpreted or displayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Erc20Units {
    /// Query the token's `decimals()` function.
    Auto,
    /// Use an explicit decimal count.
    Decimals(u8),
}

impl FromStr for Erc20Units {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }

        let decimals = value.parse::<u8>().map_err(|_| {
            format!("invalid units `{value}`; expected `auto` or a decimal count from 0 to 77")
        })?;
        if Unit::new(decimals).is_none() {
            return Err(format!(
                "invalid units `{value}`; expected `auto` or a decimal count from 0 to 77"
            ));
        }
        Ok(Self::Decimals(decimals))
    }
}

const AUTO_UNITS_CONTEXT: &str = "failed to query ERC-20 decimals() for `--units auto`; use \
    `--units <DECIMALS>` or omit `--units` for raw amounts";

fn decode_erc20_decimals(output: &[u8]) -> eyre::Result<u8> {
    if output.len() != 32 || output[..31].iter().any(|byte| *byte != 0) {
        eyre::bail!("decimals() returned non-standard ABI data; expected one uint8 word");
    }
    Ok(output[31])
}

fn validate_erc20_decimals(decimals: u8) -> eyre::Result<u8> {
    if Unit::new(decimals).is_none() {
        eyre::bail!(
            "ERC-20 decimals() returned {decimals}, but at most 77 decimals are supported; omit \
             `--units` to use raw amounts"
        );
    }
    Ok(decimals)
}

fn normalize_decimal_amount(value: &str) -> Option<String> {
    let (integer, fractional) = value.split_once('.').unwrap_or((value, ""));
    if (integer.is_empty() && fractional.is_empty())
        || integer.bytes().any(|byte| !byte.is_ascii_digit())
        || fractional.bytes().any(|byte| !byte.is_ascii_digit())
    {
        return None;
    }

    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fractional = fractional.trim_end_matches('0');
    if fractional.is_empty() {
        Some(integer.to_string())
    } else {
        Some(format!("{integer}.{fractional}"))
    }
}

fn parse_erc20_amount(amount: &str, decimals: u8) -> eyre::Result<U256> {
    let normalized = normalize_decimal_amount(amount).ok_or_else(|| {
        eyre::eyre!("invalid ERC-20 amount `{amount}`; expected an unsigned ASCII decimal")
    })?;

    if let Some((_, fractional)) = amount.split_once('.')
        && fractional
            .as_bytes()
            .get(decimals as usize..)
            .is_some_and(|extra| extra.iter().any(|byte| *byte != b'0'))
    {
        eyre::bail!(
            "ERC-20 amount `{amount}` has more than {decimals} decimal places and would lose \
             precision"
        );
    }

    let parsed = SimpleCast::parse_units(amount, decimals)
        .wrap_err_with(|| format!("invalid ERC-20 amount `{amount}` for {decimals} decimals"))?;
    let parsed = U256::from_str(&parsed).wrap_err("ERC-20 amounts must be unsigned")?;
    let formatted = SimpleCast::format_units(&parsed.to_string(), decimals)?;
    if normalized != normalize_decimal_amount(&formatted).expect("formatted amount is valid") {
        eyre::bail!(
            "ERC-20 amount `{amount}` cannot be represented exactly with {decimals} decimals"
        );
    }
    Ok(parsed)
}

/// Decimal unit options for ERC-20 amounts.
#[derive(Args, Clone, Copy, Debug, Default)]
pub struct Erc20UnitsOpts {
    /// Interpret the amount using token decimals.
    ///
    /// Pass an explicit decimal count, or `auto` to query the token's `decimals()` function.
    /// Without this option, amounts are raw integers in the token's smallest unit.
    /// Automatic mode fails if `decimals()` is missing, reverts, or returns non-standard data.
    #[arg(long, value_name = "DECIMALS|auto")]
    units: Option<Erc20Units>,
}

impl Erc20UnitsOpts {
    const fn is_auto(&self) -> bool {
        matches!(self.units, Some(Erc20Units::Auto))
    }

    async fn decimals<P, N>(
        &self,
        token: &IERC20::IERC20Instance<P, N>,
        block: BlockId,
        overrides: Option<&CallOverrideOpts>,
    ) -> eyre::Result<Option<u8>>
    where
        P: Provider<N>,
        N: Network,
    {
        match self.units {
            None => return Ok(None),
            Some(Erc20Units::Decimals(decimals)) => return Ok(Some(decimals)),
            Some(Erc20Units::Auto) => {}
        };

        let raw_decoder = ();
        let call = token.decimals().block(block).call().with_decoder(&raw_decoder);
        let output =
            if let Some(overrides) = overrides { overrides.apply(call)?.await } else { call.await }
                .wrap_err(AUTO_UNITS_CONTEXT)?;
        let decimals = decode_erc20_decimals(&output).wrap_err(AUTO_UNITS_CONTEXT)?;
        validate_erc20_decimals(decimals).map(Some)
    }

    async fn parse_amount<P, N>(
        &self,
        amount: &str,
        token: &IERC20::IERC20Instance<P, N>,
    ) -> eyre::Result<U256>
    where
        P: Provider<N>,
        N: Network,
    {
        let Some(units) = self.units else {
            return U256::from_str(amount).wrap_err("invalid raw ERC-20 amount");
        };
        if normalize_decimal_amount(amount).is_none() {
            eyre::bail!("invalid ERC-20 amount `{amount}`; expected an unsigned ASCII decimal");
        }
        let decimals = match units {
            Erc20Units::Auto => self
                .decimals(token, BlockId::default(), None)
                .await?
                .expect("auto units always resolve decimals"),
            Erc20Units::Decimals(decimals) => decimals,
        };
        parse_erc20_amount(amount, decimals)
    }

    async fn format_amount<P, N>(
        &self,
        amount: U256,
        token: &IERC20::IERC20Instance<P, N>,
        block: BlockId,
        overrides: Option<&CallOverrideOpts>,
    ) -> eyre::Result<Option<String>>
    where
        P: Provider<N>,
        N: Network,
    {
        let Some(decimals) = self.decimals(token, block, overrides).await? else {
            return Ok(None);
        };
        SimpleCast::format_units(&amount.to_string(), decimals)
            .map(Some)
            .wrap_err_with(|| format!("failed to format ERC-20 amount with {decimals} decimals"))
    }
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
        units: Erc20UnitsOpts,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        overrides: CallOverrideOpts,
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
        units: Erc20UnitsOpts,

        #[command(flatten)]
        rpc: RpcOpts,
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
        units: Erc20UnitsOpts,

        #[command(flatten)]
        rpc: RpcOpts,
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
    const fn units_opts(&self) -> Option<&Erc20UnitsOpts> {
        match self {
            Self::Allowance { units, .. }
            | Self::Approve { units, .. }
            | Self::Balance { units, .. }
            | Self::Burn { units, .. }
            | Self::Mint { units, .. }
            | Self::TotalSupply { units, .. }
            | Self::Transfer { units, .. } => Some(units),
            Self::Name { .. } | Self::Symbol { .. } | Self::Decimals { .. } => None,
        }
    }

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
        if self.rpc_opts().curl && self.units_opts().is_some_and(Erc20UnitsOpts::is_auto) {
            eyre::bail!(
                "`--units auto` cannot be used with `--curl`; use `--units <DECIMALS>` or omit \
                 `--units` for raw amounts"
            );
        }

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

                let token = IERC20::new(token, &provider);
                let block = block.unwrap_or_default();
                let allowance = token.allowance(owner, spender).block(block).call().await?;
                let formatted = units.format_amount(allowance, &token, block, None).await?;

                if shell::is_json() {
                    print_json_success(formatted.unwrap_or_else(|| allowance.to_string()))?;
                } else if let Some(formatted) = formatted {
                    sh_println!("{formatted}")?;
                } else {
                    sh_println!("{}", format_uint_exp(allowance))?;
                }
            }
            Self::Balance { token, owner, block, units, overrides, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;

                let token = IERC20::new(token, &provider);
                let block = block.unwrap_or_default();
                let balance_call = token.balanceOf(owner).block(block);
                let call = balance_call.call();
                let balance = overrides.apply(call)?.await?;
                let formatted =
                    units.format_amount(balance, &token, block, Some(&overrides)).await?;

                if shell::is_json() {
                    print_json_success(formatted.unwrap_or_else(|| balance.to_string()))?;
                } else if let Some(formatted) = formatted {
                    sh_println!("{formatted}")?;
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
                print_scalar(decimals)?;
            }
            Self::TotalSupply { token, block, units, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let token = IERC20::new(token, &provider);
                let block = block.unwrap_or_default();
                let total_supply = token.totalSupply().block(block).call().await?;
                let formatted = units.format_amount(total_supply, &token, block, None).await?;

                if shell::is_json() {
                    print_json_success(formatted.unwrap_or_else(|| total_supply.to_string()))?;
                } else if let Some(formatted) = formatted {
                    sh_println!("{formatted}")?;
                } else {
                    sh_println!("{}", format_uint_exp(total_supply))?
                }
            }
            // State-changing
            Self::Transfer { token, to, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    erc20.transfer(
                        to.resolve(&provider).await?,
                        units.parse_amount(&amount, &erc20).await?,
                    )
                })
            }
            Self::Approve { token, spender, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    erc20.approve(
                        spender.resolve(&provider).await?,
                        units.parse_amount(&amount, &erc20).await?,
                    )
                })
            }
            Self::Mint { token, to, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    erc20.mint(
                        to.resolve(&provider).await?,
                        units.parse_amount(&amount, &erc20).await?,
                    )
                })
            }
            Self::Burn { token, amount, units, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, _provider| {
                    erc20.burn(units.parse_amount(&amount, &erc20).await?)
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
        let request = if browser && chain.is_tempo() {
            tx.browser_wallet_gas_estimation_request()
        } else {
            tx.clone()
        };
        let estimated = provider.estimate_gas(request).await?;
        tx.set_gas_limit(estimated);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erc20_units_validate_boundaries() {
        assert_eq!("0".parse::<Erc20Units>().unwrap(), Erc20Units::Decimals(0));
        assert_eq!("77".parse::<Erc20Units>().unwrap(), Erc20Units::Decimals(77));
        assert!("78".parse::<Erc20Units>().is_err());

        assert_eq!(parse_erc20_amount("1.000", 0).unwrap(), U256::from(1));
        assert_eq!(parse_erc20_amount("1.23000", 2).unwrap(), U256::from(123));
        assert!(parse_erc20_amount("1.001", 2).is_err());

        let one_e77 = U256::from_str(&format!("1{}", "0".repeat(77))).unwrap();
        assert_eq!(parse_erc20_amount("1", 77).unwrap(), one_e77);
        assert!(parse_erc20_amount("2", 77).is_err());

        assert_eq!(parse_erc20_amount(&U256::MAX.to_string(), 0).unwrap(), U256::MAX);
    }

    #[test]
    fn erc20_units_reject_malformed_amounts() {
        for amount in ["", ".", "1.💥", "１", "1..0", "-1", "1_0"] {
            let error = parse_erc20_amount(amount, 1).unwrap_err().to_string();
            assert!(
                error.contains("expected an unsigned ASCII decimal"),
                "unexpected error for {amount:?}: {error}"
            );
        }
    }

    #[test]
    fn erc20_units_validate_metadata_encoding() {
        let mut word = vec![0; 32];
        word[31] = 77;
        assert_eq!(decode_erc20_decimals(&word).unwrap(), 77);

        let mut high_padding = vec![0; 32];
        high_padding[0] = 1;
        for malformed in [Vec::new(), vec![0; 31], vec![0; 64], high_padding] {
            assert!(decode_erc20_decimals(&malformed).is_err());
        }

        let error = validate_erc20_decimals(78).unwrap_err().to_string();
        assert!(error.contains("omit `--units` to use raw amounts"));
        assert!(!error.contains("--units <DECIMALS>"));
    }
}
