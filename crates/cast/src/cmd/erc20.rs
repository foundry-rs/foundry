use std::str::FromStr;

use crate::{
    cmd::send::cast_send,
    format_uint_exp,
    tx::{SendTxOpts, signing_provider},
};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_primitives::U256;
use alloy_sol_types::sol;
use clap::Parser;
use foundry_cli::{
    opts::RpcOpts,
    utils::{LoadConfig, get_provider},
};
#[doc(hidden)]
pub use foundry_config::utils::*;

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
    #[command(visible_alias = "t")]
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
    },
}

impl Erc20Subcommand {
    fn rpc(&self) -> &RpcOpts {
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

    pub async fn run(self) -> eyre::Result<()> {
        let config = self.rpc().load_config()?;

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

                sh_println!("{}", format_uint_exp(allowance))?
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
                sh_println!("{}", format_uint_exp(balance))?
            }
            Self::Name { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let name = IERC20::new(token, &provider)
                    .name()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", name)?
            }
            Self::Symbol { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let symbol = IERC20::new(token, &provider)
                    .symbol()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", symbol)?
            }
            Self::Decimals { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let decimals = IERC20::new(token, &provider)
                    .decimals()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", decimals)?
            }
            Self::TotalSupply { token, block, .. } => {
                let provider = get_provider(&config)?;
                let token = token.resolve(&provider).await?;

                let total_supply = IERC20::new(token, &provider)
                    .totalSupply()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", format_uint_exp(total_supply))?
            }
            // State-changing
            Self::Transfer { token, to, amount, send_tx, .. } => {
                let provider = signing_provider(&send_tx).await?;
                let tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .transfer(to.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();
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
            Self::Approve { token, spender, amount, send_tx, .. } => {
                let provider = signing_provider(&send_tx).await?;
                let tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .approve(spender.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();
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
            Self::Mint { token, to, amount, send_tx, .. } => {
                let provider = signing_provider(&send_tx).await?;
                let tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .mint(to.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();
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
            Self::Burn { token, amount, send_tx, .. } => {
                let provider = signing_provider(&send_tx).await?;
                let tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .burn(U256::from_str(&amount)?)
                    .into_transaction_request();
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
