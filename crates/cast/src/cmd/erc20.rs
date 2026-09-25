use foundry_common::fmt::format_uint_exp;
use std::str::FromStr;

use crate::{
    cmd::{call_overrides::CallOverrideOpts, rpc_provider, send::SendTxArgs},
    tx::{SendTxOpts, TxParams},
};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, U256};
use alloy_sol_types::{SolCall, sol};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::{print_json_success, print_scalar},
    opts::RpcOpts,
};
use foundry_common::{provider::RetryProvider, shell};

mod permit;

sol! {
    #[sol(rpc)]
    interface IERC20 {
        event Transfer(address indexed from, address indexed to, uint256 value);

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
    /// Sign an ERC-2612 approval without sending a transaction, or submit it with --broadcast.
    ///
    /// The owner is the signing wallet. Amounts are in raw token units and the deadline is an
    /// absolute Unix timestamp in seconds. This does not transfer tokens or deposit into a vault.
    /// For deposits, the permit must target the underlying asset, not the vault's share token.
    ///
    /// By default stdout contains the 65-byte signature (r, s, v). With --json, it contains the
    /// signature, permit calldata, owner, spender, value, nonce, deadline, token, and typed data.
    /// With --broadcast, output follows cast send: a receipt, or a transaction hash with --async.
    /// Anyone can submit the generated calldata to the token before the deadline.
    /// Transaction options require --broadcast; --nonce selects the transaction nonce, while the
    /// permit nonce is always read from the token.
    ///
    /// Uses EIP-5267 domain discovery when available. Otherwise uses name(), version "1", the
    /// RPC chain ID, and the token address. --domain-name and --domain-version override the name
    /// and version. The resulting domain must match DOMAIN_SEPARATOR() before signing.
    /// DAI-style permits and Permit2 are not supported.
    ///
    /// Example:
    /// ```text
    /// cast erc20 permit $TOKEN $SPENDER 1000000 --deadline $DEADLINE \
    ///     --account owner --rpc-url $RPC_URL --json
    /// cast erc20 permit $TOKEN $SPENDER 1000000 --deadline $DEADLINE \
    ///     --account owner --rpc-url $RPC_URL --broadcast --async
    /// ```
    #[command(verbatim_doc_comment)]
    Permit(permit::PermitArgs),

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
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Permit(args) => args.run().await,
            Self::Allowance { token, owner, spender, block, rpc } => {
                let (provider, erc20) = token_at(&rpc, token).await?;
                let owner = owner.resolve(&provider).await?;
                let spender = spender.resolve(&provider).await?;
                let allowance =
                    erc20.allowance(owner, spender).block(block.unwrap_or_default()).call().await?;
                print_amount(allowance)
            }
            Self::Balance { token, owner, block, rpc, overrides } => {
                let (provider, erc20) = token_at(&rpc, token).await?;
                let owner = owner.resolve(&provider).await?;
                let call = erc20.balanceOf(owner).block(block.unwrap_or_default());
                let balance = overrides.apply(call.call())?.await?;
                print_scalar(balance.to_string())
            }
            Self::Name { token, block, rpc } => {
                let (_, erc20) = token_at(&rpc, token).await?;
                print_scalar(erc20.name().block(block.unwrap_or_default()).call().await?)
            }
            Self::Symbol { token, block, rpc } => {
                let (_, erc20) = token_at(&rpc, token).await?;
                print_scalar(erc20.symbol().block(block.unwrap_or_default()).call().await?)
            }
            Self::Decimals { token, block, rpc } => {
                let (_, erc20) = token_at(&rpc, token).await?;
                print_scalar(erc20.decimals().block(block.unwrap_or_default()).call().await?)
            }
            Self::TotalSupply { token, block, rpc } => {
                let (_, erc20) = token_at(&rpc, token).await?;
                print_amount(erc20.totalSupply().block(block.unwrap_or_default()).call().await?)
            }
            Self::Transfer { token, to, amount, send_tx, tx } => {
                let to = resolve(&send_tx.eth.rpc, to).await?;
                let call = IERC20::transferCall { to, amount: U256::from_str(&amount)? };
                send(token, call, send_tx, tx).await
            }
            Self::Approve { token, spender, amount, send_tx, tx } => {
                let spender = resolve(&send_tx.eth.rpc, spender).await?;
                let call = IERC20::approveCall { spender, amount: U256::from_str(&amount)? };
                send(token, call, send_tx, tx).await
            }
            Self::Mint { token, to, amount, send_tx, tx } => {
                let to = resolve(&send_tx.eth.rpc, to).await?;
                let call = IERC20::mintCall { to, amount: U256::from_str(&amount)? };
                send(token, call, send_tx, tx).await
            }
            Self::Burn { token, amount, send_tx, tx } => {
                send(token, IERC20::burnCall { amount: U256::from_str(&amount)? }, send_tx, tx)
                    .await
            }
        }
    }
}

async fn token_at(
    rpc: &RpcOpts,
    token: NameOrAddress,
) -> Result<(RetryProvider, IERC20::IERC20Instance<RetryProvider, AnyNetwork>)> {
    let provider = rpc_provider(rpc)?;
    let token = token.resolve(&provider).await?;
    Ok((provider.clone(), IERC20::new(token, provider)))
}

async fn resolve(rpc: &RpcOpts, account: NameOrAddress) -> Result<Address> {
    Ok(account.resolve(&rpc_provider(rpc)?).await?)
}

async fn send(
    token: NameOrAddress,
    call: impl SolCall,
    send_tx: SendTxOpts,
    tx: TxParams,
) -> Result<()> {
    // Boxed to keep the large `cast send` future off this command's stack frame.
    Box::pin(SendTxArgs::contract_call(token, call.abi_encode(), send_tx, tx).run()).await
}

/// Prints a token amount: the raw decimal string in JSON mode, exponent-annotated otherwise.
pub(crate) fn print_amount(amount: U256) -> Result<()> {
    if shell::is_json() {
        print_json_success(amount.to_string())
    } else {
        sh_println!("{}", format_uint_exp(amount))
    }
}
