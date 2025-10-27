use std::str::FromStr;

use crate::Cast;
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_primitives::U256;
use clap::Parser;
use foundry_cli::{
    opts::RpcOpts,
    utils::{LoadConfig, get_provider},
};
use foundry_common::fmt::format_uint_exp;
use foundry_wallets::WalletOpts;

#[doc(hidden)]
pub use foundry_config::utils::*;

/// Interact with ERC20 tokens.
#[derive(Debug, Parser, Clone)]
pub enum Erc20Subcommand {
    /// Query ERC20 token balance.
    #[command(visible_alias = "b")]
    Balance {
        /// The ERC20 token contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        token: NameOrAddress,

        /// The account to query.
        #[arg(value_parser = NameOrAddress::from_str)]
        account: NameOrAddress,

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

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        wallet: WalletOpts,
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

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,

        #[command(flatten)]
        wallet: WalletOpts,
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
}

impl Erc20Subcommand {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            // TODO: change account for who???
            Self::Balance { token, account, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let account_addr = account.resolve(&provider).await?;
                let token_addr = token.resolve(&provider).await?;
                let balance =
                    Cast::new(&provider).erc20_balance(token_addr, account_addr, block).await?;
                sh_println!("{}", format_uint_exp(balance))?
            }
            Self::Transfer { token, to, amount, block: _, rpc, wallet } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let to_addr = to.resolve(&provider).await?;
                let token_addr = token.resolve(&provider).await?;
                let amount_u256 = U256::from_str(&amount)?;

                // Create signer from wallet options if available
                let signer = wallet.signer().await.ok();

                let tx_hash = Cast::new(&provider)
                    .erc20_transfer(token_addr, to_addr, amount_u256, signer)
                    .await?;
                sh_println!("{}", tx_hash)?
            }
            Self::Approve { token, spender, amount, block: _, rpc, wallet } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let spender_addr = spender.resolve(&provider).await?;
                let token_addr = token.resolve(&provider).await?;
                let amount_u256 = U256::from_str(&amount)?;

                // Create signer from wallet options if available
                let signer = wallet.signer().await.ok();

                let tx_hash = Cast::new(&provider)
                    .erc20_approve(token_addr, spender_addr, amount_u256, signer)
                    .await?;
                sh_println!("{}", tx_hash)?
            }
            Self::Allowance { token, owner, spender, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let owner_addr = owner.resolve(&provider).await?;
                let spender_addr = spender.resolve(&provider).await?;
                let token_addr = token.resolve(&provider).await?;

                let allowance = Cast::new(&provider)
                    .erc20_allowance(token_addr, owner_addr, spender_addr, block)
                    .await?;
                sh_println!("{}", format_uint_exp(allowance))?
            }
        }
        Ok(())
    }
}
