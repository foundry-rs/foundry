use std::str::FromStr;

use crate::{
    cmd::send::SendTxArgs,
    format_uint_exp,
    tx::{SendTxOpts, TxParams},
};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_primitives::{Address, U256, address, hex};
use alloy_sol_types::{SolCall, sol};
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    json::{print_json_success, print_scalar},
    opts::RpcOpts,
    utils::{LoadConfig, get_provider},
};
use foundry_common::shell;

const NATIVE_ASSET: Address = address!("EeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE");

sol! {
    #[sol(rpc)]
    interface IERC4626 {
        function asset() external view returns (address assetTokenAddress);
        function totalAssets() external view returns (uint256 totalManagedAssets);
        function convertToShares(uint256 assets) external view returns (uint256 shares);
        function convertToAssets(uint256 shares) external view returns (uint256 assets);
        function maxDeposit(address receiver) external view returns (uint256 maxAssets);
        function previewDeposit(uint256 assets) external view returns (uint256 shares);
        function deposit(uint256 assets, address receiver) external returns (uint256 shares);
        function maxMint(address receiver) external view returns (uint256 maxShares);
        function previewMint(uint256 shares) external view returns (uint256 assets);
        function mint(uint256 shares, address receiver) external returns (uint256 assets);
        function maxWithdraw(address owner) external view returns (uint256 maxAssets);
        function previewWithdraw(uint256 assets) external view returns (uint256 shares);
        function withdraw(uint256 assets, address receiver, address owner)
            external
            returns (uint256 shares);
        function maxRedeem(address owner) external view returns (uint256 maxShares);
        function previewRedeem(uint256 shares) external view returns (uint256 assets);
        function redeem(uint256 shares, address receiver, address owner)
            external
            returns (uint256 assets);

        function balanceOf(address owner) external view returns (uint256 shares);
    }
}

/// Interact with synchronous ERC-4626 tokenized vaults.
#[derive(Debug, Parser, Clone)]
pub enum Erc4626Subcommand {
    /// Query the vault's underlying asset token.
    Asset {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query the total amount of underlying assets managed by the vault.
    TotalAssets {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Convert an asset amount to the corresponding share amount.
    ConvertToShares {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of underlying assets.
        assets: U256,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Convert a share amount to the corresponding asset amount.
    ConvertToAssets {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of vault shares.
        shares: U256,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query the maximum assets that may be deposited for a receiver.
    MaxDeposit {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The receiver of the resulting vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        receiver: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Preview the shares received by depositing an asset amount.
    PreviewDeposit {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of underlying assets.
        assets: U256,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Deposit assets into the vault.
    Deposit {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of underlying assets.
        assets: U256,

        /// The receiver of the resulting vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        receiver: NameOrAddress,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Query the maximum shares that may be minted for a receiver.
    MaxMint {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The receiver of the resulting vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        receiver: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Preview the assets required to mint a share amount.
    PreviewMint {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of vault shares.
        shares: U256,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Mint vault shares.
    Mint {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of vault shares.
        shares: U256,

        /// The receiver of the resulting vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        receiver: NameOrAddress,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Query the maximum assets that an owner may withdraw.
    MaxWithdraw {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The owner of the vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Preview the shares burned to withdraw an asset amount.
    PreviewWithdraw {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of underlying assets.
        assets: U256,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Withdraw assets from the vault.
    Withdraw {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of underlying assets.
        assets: U256,

        /// The receiver of the withdrawn assets.
        #[arg(value_parser = NameOrAddress::from_str)]
        receiver: NameOrAddress,

        /// The owner of the vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Query the maximum shares that an owner may redeem.
    MaxRedeem {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The owner of the vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Preview the assets received by redeeming a share amount.
    PreviewRedeem {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of vault shares.
        shares: U256,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Redeem vault shares for assets.
    Redeem {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The amount of vault shares.
        shares: U256,

        /// The receiver of the redeemed assets.
        #[arg(value_parser = NameOrAddress::from_str)]
        receiver: NameOrAddress,

        /// The owner of the vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },
}

impl Erc4626Subcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Asset { vault, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let asset = IERC4626::new(vault, &provider)
                    .asset()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                warn_if_native_asset(asset)?;
                print_scalar(asset.to_string())
            }
            Self::TotalAssets { vault, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let assets = IERC4626::new(vault, &provider)
                    .totalAssets()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                print_amount(assets)
            }
            Self::ConvertToShares { vault, assets, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let shares = IERC4626::new(vault, &provider)
                    .convertToShares(assets)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                print_amount(shares)
            }
            Self::ConvertToAssets { vault, shares, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let assets = IERC4626::new(vault, &provider)
                    .convertToAssets(shares)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                print_amount(assets)
            }
            Self::MaxDeposit { vault, receiver, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let receiver = receiver.resolve(&provider).await?;
                let assets = IERC4626::new(vault, &provider)
                    .maxDeposit(receiver)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                warn_if_zero_entry_max("maxDeposit", assets)?;
                print_amount(assets)
            }
            Self::PreviewDeposit { vault, assets, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let shares = IERC4626::new(vault, &provider)
                    .previewDeposit(assets)
                    .block(block.unwrap_or_default())
                    .call()
                    .await
                    .wrap_err(
                        "previewDeposit failed; asynchronous ERC-7540 deposit vaults intentionally \
                         revert this preview",
                    )?;
                print_amount(shares)
            }
            Self::Deposit { vault, assets, receiver, send_tx, tx } => {
                let addresses = prepare_write(&vault, &[receiver], &send_tx).await?;
                send_call(
                    vault,
                    IERC4626::depositCall { assets, receiver: addresses[0] },
                    send_tx,
                    tx,
                )
                .await
            }
            Self::MaxMint { vault, receiver, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let receiver = receiver.resolve(&provider).await?;
                let shares = IERC4626::new(vault, &provider)
                    .maxMint(receiver)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                warn_if_zero_entry_max("maxMint", shares)?;
                print_amount(shares)
            }
            Self::PreviewMint { vault, shares, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let assets = IERC4626::new(vault, &provider)
                    .previewMint(shares)
                    .block(block.unwrap_or_default())
                    .call()
                    .await
                    .wrap_err(
                        "previewMint failed; asynchronous ERC-7540 deposit vaults intentionally \
                         revert this preview",
                    )?;
                print_amount(assets)
            }
            Self::Mint { vault, shares, receiver, send_tx, tx } => {
                let addresses = prepare_write(&vault, &[receiver], &send_tx).await?;
                send_call(vault, IERC4626::mintCall { shares, receiver: addresses[0] }, send_tx, tx)
                    .await
            }
            Self::MaxWithdraw { vault, owner, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;
                let block = block.unwrap_or_default();
                let contract = IERC4626::new(vault, &provider);
                let assets = contract.maxWithdraw(owner).block(block).call().await?;
                if assets.is_zero()
                    && contract
                        .balanceOf(owner)
                        .block(block)
                        .call()
                        .await
                        .is_ok_and(|shares| !shares.is_zero())
                {
                    warn_if_zero_exit_max("maxWithdraw")?;
                }
                print_amount(assets)
            }
            Self::PreviewWithdraw { vault, assets, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let shares = IERC4626::new(vault, &provider)
                    .previewWithdraw(assets)
                    .block(block.unwrap_or_default())
                    .call()
                    .await
                    .wrap_err(
                        "previewWithdraw failed; asynchronous ERC-7540 redeem vaults intentionally \
                         revert this preview",
                    )?;
                print_amount(shares)
            }
            Self::Withdraw { vault, assets, receiver, owner, send_tx, tx } => {
                let addresses = prepare_write(&vault, &[receiver, owner], &send_tx).await?;
                send_call(
                    vault,
                    IERC4626::withdrawCall { assets, receiver: addresses[0], owner: addresses[1] },
                    send_tx,
                    tx,
                )
                .await
            }
            Self::MaxRedeem { vault, owner, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;
                let block = block.unwrap_or_default();
                let contract = IERC4626::new(vault, &provider);
                let shares = contract.maxRedeem(owner).block(block).call().await?;
                if shares.is_zero()
                    && contract
                        .balanceOf(owner)
                        .block(block)
                        .call()
                        .await
                        .is_ok_and(|balance| !balance.is_zero())
                {
                    warn_if_zero_exit_max("maxRedeem")?;
                }
                print_amount(shares)
            }
            Self::PreviewRedeem { vault, shares, block, rpc } => {
                let config = rpc.load_config()?;
                let provider = get_provider(&config)?;
                let vault = vault.resolve(&provider).await?;
                let assets = IERC4626::new(vault, &provider)
                    .previewRedeem(shares)
                    .block(block.unwrap_or_default())
                    .call()
                    .await
                    .wrap_err(
                        "previewRedeem failed; asynchronous ERC-7540 redeem vaults intentionally \
                         revert this preview",
                    )?;
                print_amount(assets)
            }
            Self::Redeem { vault, shares, receiver, owner, send_tx, tx } => {
                let addresses = prepare_write(&vault, &[receiver, owner], &send_tx).await?;
                send_call(
                    vault,
                    IERC4626::redeemCall { shares, receiver: addresses[0], owner: addresses[1] },
                    send_tx,
                    tx,
                )
                .await
            }
        }
    }
}

fn print_amount(amount: U256) -> Result<()> {
    if shell::is_json() {
        print_json_success(amount.to_string())
    } else {
        sh_println!("{}", format_uint_exp(amount))
    }
}

fn warn_if_zero_entry_max(method: &str, amount: U256) -> Result<()> {
    if amount.is_zero() {
        sh_warn!(
            "Vault reported zero from {method}; some ERC-4626 vaults intentionally return \
             conservative maxima or gate deposits, so this may not mean deposits are impossible."
        )?;
    }
    Ok(())
}

fn warn_if_zero_exit_max(method: &str) -> Result<()> {
    sh_warn!(
        "Vault reported zero from {method} even though the owner has shares; liquidity, gates, \
         withdrawal queues, or a conservative implementation may prevent the base ERC-4626 exit."
    )
}

fn warn_if_native_asset(asset: Address) -> Result<()> {
    if asset == NATIVE_ASSET {
        sh_warn!(
            "Vault uses the ERC-7535 native-asset sentinel; base ERC-4626 write commands do not \
             attach native value, so use `cast send --value` when the vault requires it."
        )?;
    }
    Ok(())
}

async fn prepare_write(
    vault: &NameOrAddress,
    accounts: &[NameOrAddress],
    send_tx: &SendTxOpts,
) -> Result<Vec<Address>> {
    let config = send_tx.eth.rpc.load_config()?;
    let provider = get_provider(&config)?;
    let vault = vault.resolve(&provider).await?;

    if let Ok(asset) = IERC4626::new(vault, &provider).asset().call().await {
        warn_if_native_asset(asset)?;
    }

    let mut resolved = Vec::with_capacity(accounts.len());
    for account in accounts {
        resolved.push(account.resolve(&provider).await?);
    }
    Ok(resolved)
}

async fn send_call<C: SolCall>(
    vault: NameOrAddress,
    call: C,
    send_tx: SendTxOpts,
    tx: TxParams,
) -> Result<()> {
    let data = hex::encode_prefixed(call.abi_encode());
    SendTxArgs::contract_call(vault, data, send_tx, tx).run().await
}
