use std::{str::FromStr, time::Duration};

use crate::{
    cmd::send::{cast_send, cast_send_with_access_key},
    format_uint_exp,
    tx::{CastTxSender, SendTxOpts, TxParams},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network, TransactionBuilder};
use alloy_primitives::{Address, U256};
use alloy_provider::{Provider, fillers::RecommendedFillers};
use alloy_signer::Signature;
use alloy_sol_types::sol;
use clap::Parser;
use foundry_cli::{
    opts::RpcOpts,
    utils::{LoadConfig, get_chain, get_provider},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::{ProviderBuilder, RetryProviderWithSigner},
    shell,
    tempo::TEMPO_BROWSER_GAS_BUFFER,
};
#[doc(hidden)]
pub use foundry_config::{Chain, utils::*};
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::PATH_USD_ADDRESS;

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
        tempo_access_key: &Option<TempoAccessKeyConfig>,
    ) -> eyre::Result<bool> {
        if self.erc20_opts().is_some_and(|erc20| erc20.tempo.is_tempo())
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

    pub async fn run(self) -> eyre::Result<()> {
        // Resolve the signer once for state-changing variants.
        let (signer, tempo_access_key) = match &self {
            Self::Transfer { send_tx, .. }
            | Self::Approve { send_tx, .. }
            | Self::Mint { send_tx, .. }
            | Self::Burn { send_tx, .. } => {
                // Only attempt Tempo lookup if --from is set (avoids unnecessary I/O).
                if send_tx.eth.wallet.from.is_some() {
                    let (s, ak) = send_tx.eth.wallet.maybe_signer().await?;
                    (s, ak)
                } else {
                    (None, None)
                }
            }
            _ => (None, None),
        };

        let is_tempo = self.should_use_tempo_network(&tempo_access_key).await?;

        if is_tempo {
            self.run_generic::<TempoNetwork>(signer, tempo_access_key).await
        } else {
            self.run_generic::<Ethereum>(signer, None).await
        }
    }

    pub async fn run_generic<N: Network + RecommendedFillers>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        tempo_keychain: Option<TempoAccessKeyConfig>,
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
                let timeout = $send_tx.timeout.unwrap_or(config.transaction_timeout);
                if let Some(ref access_key) = tempo_keychain {
                    let signer = pre_resolved_signer
                        .as_ref()
                        .ok_or_else(|| eyre::eyre!("signer required for access key"))?;
                    let $provider =
                        ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
                    let $erc20 = IERC20::new($token.resolve(&$provider).await?, &$provider);
                    let mut tx = { $build_tx }.into_transaction_request();
                    $tx_opts.apply::<TempoNetwork>(
                        &mut tx,
                        get_chain(config.chain, &$provider).await?.is_legacy(),
                    );
                    cast_send_with_access_key(
                        &$provider,
                        tx,
                        signer,
                        access_key,
                        $send_tx.cast_async,
                        $send_tx.confirmations,
                        timeout,
                    )
                    .await?
                } else if let Some(browser) = $send_tx.browser.run::<N>().await? {
                    let $provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
                    if let Some(interval) = $send_tx.poll_interval {
                        $provider.client().set_poll_interval(Duration::from_secs(interval));
                    }
                    let $erc20 = IERC20::new($token.resolve(&$provider).await?, &$provider);
                    let mut tx = { $build_tx }.into_transaction_request();
                    let chain = get_chain(config.chain, &$provider).await?;
                    $tx_opts.apply::<N>(&mut tx, chain.is_legacy());
                    if chain.is_tempo() && tx.fee_token().is_none() {
                        tx.set_fee_token(PATH_USD_ADDRESS);
                    }
                    fill_tx(&$provider, &mut tx, browser.address(), chain).await?;
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
                    let $provider = build_provider_with_signer::<N>(&$send_tx, signer)?;
                    let $erc20 = IERC20::new($token.resolve(&$provider).await?, &$provider);
                    let mut tx = { $build_tx }.into_transaction_request();
                    $tx_opts.apply::<N>(
                        &mut tx,
                        get_chain(config.chain, &$provider).await?.is_legacy(),
                    );
                    cast_send(
                        $provider,
                        tx,
                        $send_tx.cast_async,
                        $send_tx.sync,
                        $send_tx.confirmations,
                        timeout,
                    )
                    .await?
                }
            }};
        }

        match self {
            // Read-only
            Self::Allowance { token, owner, spender, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;
                let spender = spender.resolve(&provider).await?;

                let allowance = IERC20::new(token, &provider)
                    .allowance(owner, spender)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;

                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&allowance.to_string())?)?
                } else {
                    sh_println!("{}", format_uint_exp(allowance))?
                }
            }
            Self::Balance { token, owner, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;

                let balance = IERC20::new(token, &provider)
                    .balanceOf(owner)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;

                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&balance.to_string())?)?
                } else {
                    sh_println!("{}", format_uint_exp(balance))?
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

                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&name)?)?
                } else {
                    sh_println!("{}", name)?
                }
            }
            Self::Symbol { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let symbol = IERC20::new(token, &provider)
                    .symbol()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;

                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&symbol)?)?
                } else {
                    sh_println!("{}", symbol)?
                }
            }
            Self::Decimals { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let decimals = IERC20::new(token, &provider)
                    .decimals()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&decimals)?)?
                } else {
                    sh_println!("{}", decimals)?
                }
            }
            Self::TotalSupply { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let total_supply = IERC20::new(token, &provider)
                    .totalSupply()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;

                if shell::is_json() {
                    sh_println!("{}", serde_json::to_string(&total_supply.to_string())?)?
                } else {
                    sh_println!("{}", format_uint_exp(total_supply))?
                }
            }
            // State-changing
            Self::Transfer { token, to, amount, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    erc20.transfer(to.resolve(&provider).await?, U256::from_str(&amount)?)
                })
            }
            Self::Approve { token, spender, amount, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    erc20.approve(spender.resolve(&provider).await?, U256::from_str(&amount)?)
                })
            }
            Self::Mint { token, to, amount, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    erc20.mint(to.resolve(&provider).await?, U256::from_str(&amount)?)
                })
            }
            Self::Burn { token, amount, send_tx, tx: tx_opts, .. } => {
                erc20_send!(token, send_tx, tx_opts, |erc20, provider| {
                    erc20.burn(U256::from_str(&amount)?)
                })
            }
        };
        Ok(())
    }
}

/// Fills from, chain_id, nonce, fees, and gas limit on a transaction request for the browser
/// wallet path. Mirrors the filling logic in the shared tx builder but operates on a
/// pre-built transaction request from the sol! macro rather than through the builder pipeline.
/// Only fills fields that haven't already been set by the user.
async fn fill_tx<N: Network, P: Provider<N>>(
    provider: &P,
    tx: &mut N::TransactionRequest,
    from: Address,
    chain: Chain,
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

    if legacy {
        if tx.gas_price().is_none() {
            tx.set_gas_price(provider.get_gas_price().await?);
        }
    } else if tx.max_fee_per_gas().is_none() || tx.max_priority_fee_per_gas().is_none() {
        let estimate = provider.estimate_eip1559_fees().await?;
        if tx.max_fee_per_gas().is_none() {
            tx.set_max_fee_per_gas(estimate.max_fee_per_gas);
        }
        if tx.max_priority_fee_per_gas().is_none() {
            tx.set_max_priority_fee_per_gas(estimate.max_priority_fee_per_gas);
        }
    }

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
