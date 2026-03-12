use std::{str::FromStr, time::Duration};

use crate::{cmd::send::cast_send, format_uint_exp, tx::SendTxOpts};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_network::{AnyNetwork, EthereumWallet, Network};
use alloy_primitives::{U64, U256};
use alloy_provider::{Provider, fillers::RecommendedFillers};
use alloy_signer::Signature;
use alloy_sol_types::sol;
use clap::{Args, Parser};
use foundry_cli::{
    opts::{RpcOpts, TempoOpts},
    utils::{LoadConfig, get_chain, get_provider},
};
use foundry_common::{
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::{ProviderBuilder, RetryProviderWithSigner},
    shell,
};
#[doc(hidden)]
pub use foundry_config::{Chain, utils::*};
use foundry_primitives::FoundryTransactionBuilder;
use tempo_alloy::TempoNetwork;

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

/// Transaction options for ERC20 operations.
///
/// This struct contains only the transaction options relevant to ERC20 token interactions
#[derive(Debug, Clone, Args)]
pub struct Erc20TxOpts {
    /// Gas limit for the transaction.
    #[arg(long, env = "ETH_GAS_LIMIT")]
    pub gas_limit: Option<U256>,

    /// Gas price for legacy transactions, or max fee per gas for EIP1559 transactions.
    #[arg(long, env = "ETH_GAS_PRICE")]
    pub gas_price: Option<U256>,

    /// Max priority fee per gas for EIP1559 transactions.
    #[arg(long, env = "ETH_PRIORITY_GAS_PRICE")]
    pub priority_gas_price: Option<U256>,

    /// Nonce for the transaction.
    #[arg(long)]
    pub nonce: Option<U64>,

    #[command(flatten)]
    pub tempo: TempoOpts,
}

/// Creates a provider with wallet for signing transactions locally.
pub(crate) async fn get_provider_with_wallet<N: Network + RecommendedFillers>(
    tx_opts: &SendTxOpts,
) -> eyre::Result<RetryProviderWithSigner<N>>
where
    N::TxEnvelope: From<Signed<N::UnsignedTx>>,
    N::UnsignedTx: SignableTransaction<Signature>,
{
    let config = tx_opts.eth.load_config()?;
    let signer = tx_opts.eth.wallet.signer().await?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::<N>::from_config(&config)?.build_with_wallet(wallet)?;
    if let Some(interval) = tx_opts.poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval))
    }
    Ok(provider)
}

/// Apply transaction options to a transaction request for ERC20 operations.
fn apply_tx_opts<N: Network, T: FoundryTransactionBuilder<N>>(
    tx: &mut T,
    tx_opts: &Erc20TxOpts,
    is_legacy: bool,
) {
    if let Some(gas_limit) = tx_opts.gas_limit {
        tx.set_gas_limit(gas_limit.to());
    }

    if let Some(gas_price) = tx_opts.gas_price {
        if is_legacy {
            tx.set_gas_price(gas_price.to());
        } else {
            tx.set_max_fee_per_gas(gas_price.to());
        }
    }

    if !is_legacy && let Some(priority_fee) = tx_opts.priority_gas_price {
        tx.set_max_priority_fee_per_gas(priority_fee.to());
    }

    if let Some(nonce) = tx_opts.nonce {
        tx.set_nonce(nonce.to());
    }

    // Apply Tempo-specific options
    if let Some(fee_token) = tx_opts.tempo.fee_token {
        tx.set_fee_token(fee_token);
    }

    if let Some(nonce_key) = tx_opts.tempo.sequence_key {
        tx.set_nonce_key(nonce_key);
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
        tx: Erc20TxOpts,
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
        tx: Erc20TxOpts,
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
        tx: Erc20TxOpts,
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
        tx: Erc20TxOpts,
    },
}

impl Erc20Subcommand {
    fn rpc_opts(&self) -> &RpcOpts {
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

    fn erc20_opts(&self) -> Option<&Erc20TxOpts> {
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

    pub async fn run(self) -> eyre::Result<()> {
        if let Some(erc20) = self.erc20_opts()
            && (erc20.tempo.fee_token.is_some() || erc20.tempo.sequence_key.is_some())
        {
            self.run_generic::<TempoNetwork>().await
        } else {
            self.run_generic::<AnyNetwork>().await
        }
    }

    pub async fn run_generic<N: Network + RecommendedFillers>(self) -> eyre::Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let config = self.rpc_opts().load_config()?;

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
                let provider = get_provider_with_wallet::<N>(&send_tx).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .transfer(to.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                cast_send(
                    provider,
                    tx,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
            Self::Approve { token, spender, amount, send_tx, tx: tx_opts, .. } => {
                let provider = get_provider_with_wallet::<N>(&send_tx).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .approve(spender.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                cast_send(
                    provider,
                    tx,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
            Self::Mint { token, to, amount, send_tx, tx: tx_opts, .. } => {
                let provider = get_provider_with_wallet::<N>(&send_tx).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .mint(to.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                cast_send(
                    provider,
                    tx,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
            Self::Burn { token, amount, send_tx, tx: tx_opts, .. } => {
                let provider = get_provider_with_wallet::<N>(&send_tx).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .burn(U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                cast_send(
                    provider,
                    tx,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
        };
        Ok(())
    }
}
