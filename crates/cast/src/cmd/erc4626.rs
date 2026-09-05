use std::str::FromStr;

use crate::{
    cmd::{erc20::print_amount, rpc_provider, send::SendTxArgs},
    tx::{SendTxOpts, TxParams},
};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_network::AnyNetwork;
use alloy_primitives::{Address, FixedBytes, U256, address};
use alloy_provider::Provider;
use alloy_sol_types::{SolCall, sol};
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    json::{
        JsonError, JsonMessage, print_json_success, print_json_success_with_warnings, print_scalar,
    },
    opts::RpcOpts,
};
use foundry_common::{provider::RetryProvider, shell};
use serde::Serialize;

const NATIVE_ASSET: Address = address!("EeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE");
/// ERC-7535 asset quantities are denominated in wei.
const NATIVE_ASSET_DECIMALS: u8 = 18;
const ERC7540_ASYNC_DEPOSIT_INTERFACE: FixedBytes<4> = FixedBytes::new([0xce, 0x3b, 0xbe, 0x50]);
const ERC7540_ASYNC_REDEEM_INTERFACE: FixedBytes<4> = FixedBytes::new([0x62, 0x0e, 0xe8, 0xe4]);

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

        function name() external view returns (string);
        function symbol() external view returns (string);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256 shares);
        function balanceOf(address owner) external view returns (uint256 shares);
        function allowance(address owner, address spender) external view returns (uint256 shares);
    }

    #[sol(rpc)]
    interface IERC20Metadata {
        function name() external view returns (string);
        function symbol() external view returns (string);
        function decimals() external view returns (uint8);
        function balanceOf(address owner) external view returns (uint256 amount);
    }

    #[sol(rpc)]
    interface IERC165 {
        function supportsInterface(bytes4 interfaceId) external view returns (bool);
    }
}

/// Interact with synchronous ERC-4626 tokenized vaults.
#[derive(Debug, Parser, Clone)]
pub enum Erc4626Subcommand {
    /// Show vault, asset, and exchange-rate information
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 info 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB --human \
    ///     --block 25519075 --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// Vault                0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB
    /// Name                 Steakhouse USDC
    /// Symbol               steakUSDC
    /// Decimals             18
    /// Asset                0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
    /// Asset name           USD Coin
    /// Asset symbol         USDC
    /// Asset decimals       6
    /// Total assets         95183395.377893 USDC
    /// Total supply         84037200.060143388288943211 steakUSDC
    /// Assets per share     1.132634 USDC
    /// Shares per asset     0.882897731163608580 steakUSDC
    /// ```
    #[command(verbatim_doc_comment)]
    Info {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// Use formatted token amounts in text output.
        #[arg(long)]
        human: bool,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Show an account's shares, asset value, and withdrawal limits
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 position 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     0x255c7705E8bb334dfCaE438197f7c4297988085A --human --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// Vault                0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB
    /// Owner                0x255c7705e8BB334DfCae438197f7C4297988085a
    /// Asset                0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
    /// Share symbol         steakUSDC
    /// Share decimals       18
    /// Asset symbol         USDC
    /// Asset decimals       6
    /// Share balance        35733.949295544029939485 steakUSDC
    /// Assets equivalent    40473.486378 USDC
    /// Max withdraw         40473.486378 USDC
    /// Max redeem           35733.949295417417957447 steakUSDC
    /// ```
    #[command(verbatim_doc_comment)]
    Position {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The owner of the vault shares.
        #[arg(value_parser = NameOrAddress::from_str)]
        owner: NameOrAddress,

        /// Use formatted token amounts in text output.
        #[arg(long)]
        human: bool,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Probe synchronous ERC-4626 interface compatibility
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 check 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     --account 0x255c7705E8bb334dfCaE438197f7c4297988085A --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// Vault                0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB
    /// Account              0x255c7705e8BB334DfCae438197f7C4297988085a
    /// Note: This probes read-call behavior only; it does not prove state-changing selector coverage or semantic ERC-4626 compliance.
    /// PASS contract code            contract bytecode is present
    /// PASS asset()                  returned 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
    /// PASS asset contract           underlying asset bytecode is present
    /// PASS asset balanceOf(address) call succeeded
    /// PASS totalAssets()            call succeeded
    /// PASS totalSupply()            call succeeded
    /// PASS balanceOf(address)       call succeeded
    /// PASS allowance(address,address) call succeeded
    /// PASS convertToShares(0)       returned zero
    /// PASS convertToAssets(0)       returned zero
    /// PASS maxDeposit(address)      call succeeded
    /// PASS previewDeposit(0)        returned zero
    /// PASS maxMint(address)         call succeeded
    /// PASS previewMint(0)           returned zero
    /// PASS maxWithdraw(address)     call succeeded
    /// PASS previewWithdraw(0)       returned zero
    /// PASS maxRedeem(address)       call succeeded
    /// PASS previewRedeem(0)         returned zero
    /// PASS name()                   call succeeded
    /// PASS symbol()                 call succeeded
    /// PASS decimals()               call succeeded
    /// Summary: 21 passed, 0 warnings, 0 failed
    /// ```
    #[command(verbatim_doc_comment)]
    Check {
        /// The ERC-4626 vault contract address.
        #[arg(value_parser = NameOrAddress::from_str)]
        vault: NameOrAddress,

        /// The account used for limit and balance probes.
        #[arg(long, value_parser = NameOrAddress::from_str)]
        account: Option<NameOrAddress>,

        /// The block height to query at.
        #[arg(long, short = 'B')]
        block: Option<BlockId>,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Query the vault's underlying asset token
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 asset 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     --block 25519075 --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Query the total amount of underlying assets managed by the vault
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 total-assets 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     --block 25519075 --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 95183395377893 [9.518e13]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Convert an asset amount to the corresponding share amount
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 convert-to-shares 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000 --block 25519075 --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 882897731163608580 [8.828e17]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Convert a share amount to the corresponding asset amount
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 convert-to-assets 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000000000000000 --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 1132634 [1.132e6]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Query the maximum assets that may be deposited for a receiver
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 max-deposit 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     0x255c7705E8bb334dfCaE438197f7c4297988085A --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 1002934816604622098 [1.002e18]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Preview the shares received by depositing an asset amount
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 preview-deposit 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000 --block 25519075 --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 882897731163608580 [8.828e17]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Deposit assets into the vault
    ///
    /// The vault must have sufficient allowance to spend the underlying asset.
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 deposit 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000 $ACCOUNT --private-key $PRIVATE_KEY --async --rpc-url $ETH_RPC_URL
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 0x6f2a7e10f148a0ee81208cd8d7dee10cc33b5bdb739bfeef805dc68467e6db4e
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Query the maximum shares that may be minted for a receiver
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 max-mint 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     0x255c7705E8bb334dfCaE438197f7c4297988085A --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 885488874085210715750045480687 [8.854e29]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Preview the assets required to mint a share amount
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 preview-mint 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000000000000000 --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 1132635 [1.132e6]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Mint vault shares
    ///
    /// The vault must have sufficient allowance to spend the underlying asset.
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 mint 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000000000000000 $ACCOUNT --private-key $PRIVATE_KEY --async \
    ///     --rpc-url $ETH_RPC_URL
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 0xa7c3b4e26a99e2cb422e2b36cffeae642a39f1d09f8f3454c9c2d4657ebb0491
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Query the maximum assets that an owner may withdraw
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 max-withdraw 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     0x255c7705E8bb334dfCaE438197f7c4297988085A --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 40473486378 [4.047e10]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Preview the shares burned to withdraw an asset amount
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 preview-withdraw 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000 --block 25519075 --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 882897731163608581 [8.828e17]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Withdraw assets from the vault
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 withdraw 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000 $ACCOUNT $ACCOUNT --private-key $PRIVATE_KEY --async \
    ///     --rpc-url $ETH_RPC_URL
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 0x6d6d70151151f30aa13bdb4082ea7fd3e531193eebdfbb356586284cd0e6a8a2
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Query the maximum shares that an owner may redeem
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 max-redeem 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     0x255c7705E8bb334dfCaE438197f7c4297988085A --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 35733949295417417957447 [3.573e22]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Preview the assets received by redeeming a share amount
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 preview-redeem 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000000000000000 --block 25519075 \
    ///     --rpc-url https://ethereum.reth.rs/rpc
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 1132634 [1.132e6]
    /// ```
    #[command(verbatim_doc_comment)]
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

    /// Redeem vault shares for assets
    ///
    /// Example:
    ///
    /// ```text
    /// $ cast erc4626 redeem 0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB \
    ///     1000000000000000000 $ACCOUNT $ACCOUNT --private-key $PRIVATE_KEY --async \
    ///     --rpc-url $ETH_RPC_URL
    /// ```
    ///
    /// Output:
    ///
    /// ```text
    /// 0x304d463e4d33b462778a974d008cd1b9c4c109730ae6617d2815477b8aa7c03e
    /// ```
    #[command(verbatim_doc_comment)]
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

#[derive(Debug, Serialize)]
struct TokenAmount {
    raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    formatted: Option<String>,
}

impl TokenAmount {
    fn new(value: U256, decimals: Option<u8>) -> Self {
        Self {
            raw: value.to_string(),
            formatted: decimals
                .and_then(|decimals| crate::args::format_units(&value.to_string(), decimals).ok()),
        }
    }
}

#[derive(Debug, Serialize)]
struct VaultInfo {
    vault: String,
    name: Option<String>,
    symbol: Option<String>,
    decimals: Option<u8>,
    asset: String,
    asset_name: Option<String>,
    asset_symbol: Option<String>,
    asset_decimals: Option<u8>,
    total_assets: TokenAmount,
    total_supply: TokenAmount,
    assets_per_share: Option<TokenAmount>,
    shares_per_asset: Option<TokenAmount>,
}

#[derive(Debug, Serialize)]
struct VaultPosition {
    vault: String,
    owner: String,
    asset: String,
    share_symbol: Option<String>,
    share_decimals: Option<u8>,
    asset_symbol: Option<String>,
    asset_decimals: Option<u8>,
    share_balance: TokenAmount,
    assets_equivalent: TokenAmount,
    max_withdraw: TokenAmount,
    max_redeem: TokenAmount,
}

#[derive(Debug)]
struct VaultWarning {
    code: &'static str,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Serialize)]
struct CompatibilityCheck {
    name: String,
    status: CheckStatus,
    detail: String,
}

#[derive(Debug, Serialize)]
struct CompatibilityReport {
    vault: String,
    account: String,
    read_compatible: bool,
    disclaimer: &'static str,
    passed: usize,
    warnings: usize,
    failed: usize,
    checks: Vec<CompatibilityCheck>,
}

const CHECK_DISCLAIMER: &str = "This probes read-call behavior only; it does not prove state-changing selector coverage or \
     semantic ERC-4626 compliance.";

impl Erc4626Subcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Info { vault, human, block, rpc } => show_info(vault, human, block, rpc).await,
            Self::Position { vault, owner, human, block, rpc } => {
                show_position(vault, owner, human, block, rpc).await
            }
            Self::Check { vault, account, block, rpc } => {
                check_compatibility(vault, account, block, rpc).await
            }
            Self::Asset { vault, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                let asset = vault.asset().block(block.unwrap_or_default()).call().await?;
                warn_if_native_asset(asset)?;
                print_scalar(asset.to_string())
            }
            Self::TotalAssets { vault, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                print_amount(vault.totalAssets().block(block.unwrap_or_default()).call().await?)
            }
            Self::ConvertToShares { vault, assets, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                let call = vault.convertToShares(assets).block(block.unwrap_or_default());
                print_amount(call.call().await?)
            }
            Self::ConvertToAssets { vault, shares, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                let call = vault.convertToAssets(shares).block(block.unwrap_or_default());
                print_amount(call.call().await?)
            }
            Self::MaxDeposit { vault, receiver, block, rpc } => {
                let (provider, vault) = vault_at(&rpc, vault).await?;
                let receiver = receiver.resolve(&provider).await?;
                let assets =
                    vault.maxDeposit(receiver).block(block.unwrap_or_default()).call().await?;
                warn_if_zero_entry_max("maxDeposit", assets)?;
                print_amount(assets)
            }
            Self::PreviewDeposit { vault, assets, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                let call = vault.previewDeposit(assets).block(block.unwrap_or_default());
                print_amount(call.call().await.wrap_err_with(|| preview_error("Deposit"))?)
            }
            Self::Deposit { vault, assets, receiver, send_tx, tx } => {
                let [receiver] = prepare_write(&vault, [receiver], &send_tx).await?;
                send_call(vault, IERC4626::depositCall { assets, receiver }, send_tx, tx).await
            }
            Self::MaxMint { vault, receiver, block, rpc } => {
                let (provider, vault) = vault_at(&rpc, vault).await?;
                let receiver = receiver.resolve(&provider).await?;
                let shares =
                    vault.maxMint(receiver).block(block.unwrap_or_default()).call().await?;
                warn_if_zero_entry_max("maxMint", shares)?;
                print_amount(shares)
            }
            Self::PreviewMint { vault, shares, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                let call = vault.previewMint(shares).block(block.unwrap_or_default());
                print_amount(call.call().await.wrap_err_with(|| preview_error("Mint"))?)
            }
            Self::Mint { vault, shares, receiver, send_tx, tx } => {
                let [receiver] = prepare_write(&vault, [receiver], &send_tx).await?;
                send_call(vault, IERC4626::mintCall { shares, receiver }, send_tx, tx).await
            }
            Self::MaxWithdraw { vault, owner, block, rpc } => {
                let (provider, vault) = vault_at(&rpc, vault).await?;
                let owner = owner.resolve(&provider).await?;
                let block = block.unwrap_or_default();
                let assets = vault.maxWithdraw(owner).block(block).call().await?;
                if assets.is_zero() && has_shares(&vault, owner, block).await {
                    warn_if_zero_exit_max("maxWithdraw")?;
                }
                print_amount(assets)
            }
            Self::PreviewWithdraw { vault, assets, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                let call = vault.previewWithdraw(assets).block(block.unwrap_or_default());
                print_amount(call.call().await.wrap_err_with(|| preview_error("Withdraw"))?)
            }
            Self::Withdraw { vault, assets, receiver, owner, send_tx, tx } => {
                let [receiver, owner] = prepare_write(&vault, [receiver, owner], &send_tx).await?;
                let call = IERC4626::withdrawCall { assets, receiver, owner };
                send_call(vault, call, send_tx, tx).await
            }
            Self::MaxRedeem { vault, owner, block, rpc } => {
                let (provider, vault) = vault_at(&rpc, vault).await?;
                let owner = owner.resolve(&provider).await?;
                let block = block.unwrap_or_default();
                let shares = vault.maxRedeem(owner).block(block).call().await?;
                if shares.is_zero() && has_shares(&vault, owner, block).await {
                    warn_if_zero_exit_max("maxRedeem")?;
                }
                print_amount(shares)
            }
            Self::PreviewRedeem { vault, shares, block, rpc } => {
                let (_, vault) = vault_at(&rpc, vault).await?;
                let call = vault.previewRedeem(shares).block(block.unwrap_or_default());
                print_amount(call.call().await.wrap_err_with(|| preview_error("Redeem"))?)
            }
            Self::Redeem { vault, shares, receiver, owner, send_tx, tx } => {
                let [receiver, owner] = prepare_write(&vault, [receiver, owner], &send_tx).await?;
                let call = IERC4626::redeemCall { shares, receiver, owner };
                send_call(vault, call, send_tx, tx).await
            }
        }
    }
}

type Vault = IERC4626::IERC4626Instance<RetryProvider, AnyNetwork>;

async fn vault_at(rpc: &RpcOpts, vault: NameOrAddress) -> Result<(RetryProvider, Vault)> {
    let provider = rpc_provider(rpc)?;
    let vault = vault.resolve(&provider).await?;
    Ok((provider.clone(), IERC4626::new(vault, provider)))
}

async fn has_shares(vault: &Vault, owner: Address, block: BlockId) -> bool {
    vault.balanceOf(owner).block(block).call().await.is_ok_and(|shares| !shares.is_zero())
}

/// Error context for a failed `preview*` call; asynchronous ERC-7540 vaults revert these.
fn preview_error(method: &str) -> String {
    let kind = if matches!(method, "Deposit" | "Mint") { "deposit" } else { "redeem" };
    format!(
        "preview{method} failed; asynchronous ERC-7540 {kind} vaults intentionally revert this \
         preview"
    )
}

async fn show_info(
    vault: NameOrAddress,
    human: bool,
    block: Option<BlockId>,
    rpc: RpcOpts,
) -> Result<()> {
    let (provider, contract) = vault_at(&rpc, vault).await?;
    let vault = *contract.address();
    let block = block.unwrap_or_default();

    let name_call = contract.name().block(block);
    let symbol_call = contract.symbol().block(block);
    let decimals_call = contract.decimals().block(block);
    let asset_call = contract.asset().block(block);
    let total_assets_call = contract.totalAssets().block(block);
    let total_supply_call = contract.totalSupply().block(block);
    let (name, symbol, decimals, asset, total_assets, total_supply) = tokio::join!(
        name_call.call(),
        symbol_call.call(),
        decimals_call.call(),
        asset_call.call(),
        total_assets_call.call(),
        total_supply_call.call(),
    );

    let asset = asset.wrap_err("asset() call failed")?;
    let total_assets = total_assets.wrap_err("totalAssets() call failed")?;
    let total_supply = total_supply.wrap_err("totalSupply() call failed")?;
    let name = name.ok();
    let symbol = symbol.ok();
    let decimals = decimals.ok();

    let mut warnings = Vec::new();
    let (asset_name, asset_symbol, asset_decimals) = if asset == NATIVE_ASSET {
        warnings.push(native_asset_warning());
        (None, None, Some(NATIVE_ASSET_DECIMALS))
    } else {
        let asset_contract = IERC20Metadata::new(asset, &provider);
        let name_call = asset_contract.name().block(block);
        let symbol_call = asset_contract.symbol().block(block);
        let decimals_call = asset_contract.decimals().block(block);
        let (name, symbol, decimals) =
            tokio::join!(name_call.call(), symbol_call.call(), decimals_call.call());
        (name.ok(), symbol.ok(), decimals.ok())
    };

    let assets_per_share = match decimals.and_then(decimal_unit) {
        Some(unit) => contract.convertToAssets(unit).block(block).call().await.ok(),
        None => None,
    }
    .map(|value| TokenAmount::new(value, asset_decimals));
    let shares_per_asset = match asset_decimals.and_then(decimal_unit) {
        Some(unit) => contract.convertToShares(unit).block(block).call().await.ok(),
        None => None,
    }
    .map(|value| TokenAmount::new(value, decimals));

    print_info(
        VaultInfo {
            vault: vault.to_string(),
            name,
            symbol,
            decimals,
            asset: asset.to_string(),
            asset_name,
            asset_symbol,
            asset_decimals,
            total_assets: TokenAmount::new(total_assets, asset_decimals),
            total_supply: TokenAmount::new(total_supply, decimals),
            assets_per_share,
            shares_per_asset,
        },
        human,
        warnings,
    )
}

async fn show_position(
    vault: NameOrAddress,
    owner: NameOrAddress,
    human: bool,
    block: Option<BlockId>,
    rpc: RpcOpts,
) -> Result<()> {
    let (provider, contract) = vault_at(&rpc, vault).await?;
    let vault = *contract.address();
    let owner = owner.resolve(&provider).await?;
    let block = block.unwrap_or_default();

    let asset_call = contract.asset().block(block);
    let symbol_call = contract.symbol().block(block);
    let decimals_call = contract.decimals().block(block);
    let balance_call = contract.balanceOf(owner).block(block);
    let max_withdraw_call = contract.maxWithdraw(owner).block(block);
    let max_redeem_call = contract.maxRedeem(owner).block(block);
    let (asset, share_symbol, share_decimals, share_balance, max_withdraw, max_redeem) = tokio::join!(
        asset_call.call(),
        symbol_call.call(),
        decimals_call.call(),
        balance_call.call(),
        max_withdraw_call.call(),
        max_redeem_call.call(),
    );

    let asset = asset.wrap_err("asset() call failed")?;
    let share_balance = share_balance.wrap_err("balanceOf() call failed")?;
    let max_withdraw = max_withdraw.wrap_err("maxWithdraw() call failed")?;
    let max_redeem = max_redeem.wrap_err("maxRedeem() call failed")?;
    let share_symbol = share_symbol.ok();
    let share_decimals = share_decimals.ok();
    let assets_equivalent = contract
        .convertToAssets(share_balance)
        .block(block)
        .call()
        .await
        .wrap_err("convertToAssets() call failed")?;

    let mut warnings = Vec::new();
    let (asset_symbol, asset_decimals) = if asset == NATIVE_ASSET {
        warnings.push(native_asset_warning());
        (None, Some(NATIVE_ASSET_DECIMALS))
    } else {
        let asset_contract = IERC20Metadata::new(asset, &provider);
        let symbol_call = asset_contract.symbol().block(block);
        let decimals_call = asset_contract.decimals().block(block);
        let (symbol, decimals) = tokio::join!(symbol_call.call(), decimals_call.call());
        (symbol.ok(), decimals.ok())
    };

    if !share_balance.is_zero() {
        if max_withdraw.is_zero() {
            warnings.push(zero_exit_warning("maxWithdraw"));
        }
        if max_redeem.is_zero() {
            warnings.push(zero_exit_warning("maxRedeem"));
        }
    }

    print_position(
        VaultPosition {
            vault: vault.to_string(),
            owner: owner.to_string(),
            asset: asset.to_string(),
            share_symbol,
            share_decimals,
            asset_symbol,
            asset_decimals,
            share_balance: TokenAmount::new(share_balance, share_decimals),
            assets_equivalent: TokenAmount::new(assets_equivalent, asset_decimals),
            max_withdraw: TokenAmount::new(max_withdraw, asset_decimals),
            max_redeem: TokenAmount::new(max_redeem, share_decimals),
        },
        human,
        warnings,
    )
}

async fn check_compatibility(
    vault: NameOrAddress,
    account: Option<NameOrAddress>,
    block: Option<BlockId>,
    rpc: RpcOpts,
) -> Result<()> {
    let (provider, contract) = vault_at(&rpc, vault).await?;
    let vault = *contract.address();
    let account = match account {
        Some(account) => account.resolve(&provider).await?,
        None => Address::ZERO,
    };
    let block = block.unwrap_or_default();

    let code_call = provider.get_code_at(vault).block_id(block);
    let asset_call = contract.asset().block(block);
    let total_assets_call = contract.totalAssets().block(block);
    let convert_to_shares_call = contract.convertToShares(U256::ZERO).block(block);
    let convert_to_assets_call = contract.convertToAssets(U256::ZERO).block(block);
    let max_deposit_call = contract.maxDeposit(account).block(block);
    let preview_deposit_call = contract.previewDeposit(U256::ZERO).block(block);
    let max_mint_call = contract.maxMint(account).block(block);
    let preview_mint_call = contract.previewMint(U256::ZERO).block(block);
    let max_withdraw_call = contract.maxWithdraw(account).block(block);
    let preview_withdraw_call = contract.previewWithdraw(U256::ZERO).block(block);
    let max_redeem_call = contract.maxRedeem(account).block(block);
    let preview_redeem_call = contract.previewRedeem(U256::ZERO).block(block);
    let name_call = contract.name().block(block);
    let symbol_call = contract.symbol().block(block);
    let decimals_call = contract.decimals().block(block);
    let total_supply_call = contract.totalSupply().block(block);
    let balance_call = contract.balanceOf(account).block(block);
    let allowance_call = contract.allowance(account, vault).block(block);
    let erc165 = IERC165::new(vault, &provider);
    let async_deposit_call = erc165.supportsInterface(ERC7540_ASYNC_DEPOSIT_INTERFACE).block(block);
    let async_redeem_call = erc165.supportsInterface(ERC7540_ASYNC_REDEEM_INTERFACE).block(block);
    let (
        code,
        asset,
        total_assets,
        convert_to_shares,
        convert_to_assets,
        max_deposit,
        preview_deposit,
        max_mint,
        preview_mint,
        max_withdraw,
        preview_withdraw,
        max_redeem,
        preview_redeem,
        name,
        symbol,
        decimals,
        total_supply,
        balance,
        allowance,
        async_deposit,
        async_redeem,
    ) = tokio::join!(
        code_call,
        asset_call.call(),
        total_assets_call.call(),
        convert_to_shares_call.call(),
        convert_to_assets_call.call(),
        max_deposit_call.call(),
        preview_deposit_call.call(),
        max_mint_call.call(),
        preview_mint_call.call(),
        max_withdraw_call.call(),
        preview_withdraw_call.call(),
        max_redeem_call.call(),
        preview_redeem_call.call(),
        name_call.call(),
        symbol_call.call(),
        decimals_call.call(),
        total_supply_call.call(),
        balance_call.call(),
        allowance_call.call(),
        async_deposit_call.call(),
        async_redeem_call.call(),
    );
    let async_deposit = async_deposit.unwrap_or(false);
    let async_redeem = async_redeem.unwrap_or(false);

    let mut checks = Vec::new();
    match code {
        Ok(code) if !code.is_empty() => push_check(
            &mut checks,
            "contract code",
            CheckStatus::Pass,
            "contract bytecode is present",
        ),
        Ok(_) => push_check(
            &mut checks,
            "contract code",
            CheckStatus::Fail,
            "no contract bytecode was found",
        ),
        Err(_) => push_check(
            &mut checks,
            "contract code",
            CheckStatus::Fail,
            "contract bytecode could not be read",
        ),
    }

    if let Ok(asset) = asset {
        if asset.is_zero() {
            push_check(&mut checks, "asset()", CheckStatus::Fail, "returned the zero address");
        } else if asset == NATIVE_ASSET {
            push_check(
                &mut checks,
                "asset()",
                CheckStatus::Warn,
                "returned the ERC-7535 native-asset sentinel",
            );
        } else {
            push_check(&mut checks, "asset()", CheckStatus::Pass, format!("returned {asset}"));
            match provider.get_code_at(asset).block_id(block).await {
                Ok(code) if !code.is_empty() => push_check(
                    &mut checks,
                    "asset contract",
                    CheckStatus::Pass,
                    "underlying asset bytecode is present",
                ),
                Ok(_) => push_check(
                    &mut checks,
                    "asset contract",
                    CheckStatus::Warn,
                    "underlying asset has no bytecode and may be a system contract or precompile",
                ),
                Err(_) => push_check(
                    &mut checks,
                    "asset contract",
                    CheckStatus::Warn,
                    "underlying asset bytecode could not be read",
                ),
            }
            record_required(
                &mut checks,
                "asset balanceOf(address)",
                IERC20Metadata::new(asset, &provider).balanceOf(vault).block(block).call().await,
            );
        }
    } else {
        push_check(
            &mut checks,
            "asset()",
            CheckStatus::Fail,
            "call failed or returned incompatible data",
        );
    }

    record_required(&mut checks, "totalAssets()", total_assets);
    record_required(&mut checks, "totalSupply()", total_supply);
    record_required(&mut checks, "balanceOf(address)", balance);
    record_required(&mut checks, "allowance(address,address)", allowance);
    record_zero_conversion(&mut checks, "convertToShares(0)", convert_to_shares);
    record_zero_conversion(&mut checks, "convertToAssets(0)", convert_to_assets);
    record_required(&mut checks, "maxDeposit(address)", max_deposit);
    record_preview(&mut checks, "previewDeposit(0)", "deposit", async_deposit, preview_deposit);
    record_required(&mut checks, "maxMint(address)", max_mint);
    record_preview(&mut checks, "previewMint(0)", "deposit", async_deposit, preview_mint);
    record_required(&mut checks, "maxWithdraw(address)", max_withdraw);
    record_preview(&mut checks, "previewWithdraw(0)", "redeem", async_redeem, preview_withdraw);
    record_required(&mut checks, "maxRedeem(address)", max_redeem);
    record_preview(&mut checks, "previewRedeem(0)", "redeem", async_redeem, preview_redeem);
    record_required(&mut checks, "name()", name);
    record_required(&mut checks, "symbol()", symbol);
    record_required(&mut checks, "decimals()", decimals);

    let passed = checks.iter().filter(|check| matches!(check.status, CheckStatus::Pass)).count();
    let warnings = checks.iter().filter(|check| matches!(check.status, CheckStatus::Warn)).count();
    let failed = checks.iter().filter(|check| matches!(check.status, CheckStatus::Fail)).count();
    let report = CompatibilityReport {
        vault: vault.to_string(),
        account: account.to_string(),
        read_compatible: failed == 0,
        disclaimer: CHECK_DISCLAIMER,
        passed,
        warnings,
        failed,
        checks,
    };
    if failed > 0 {
        let message = format!("vault failed {failed} ERC-4626 compatibility probe(s)");
        if shell::is_json() {
            return Err(JsonError::new(
                report,
                JsonMessage::error("erc4626.compatibility_failed", message),
            )?
            .into());
        }
        print_compatibility_report(&report)?;
        eyre::bail!(message)
    }
    print_compatibility_report(&report)
}

fn print_info(info: VaultInfo, human: bool, warnings: Vec<VaultWarning>) -> Result<()> {
    if shell::is_json() {
        return print_json_with_warnings(info, warnings);
    }

    print_warnings(&warnings)?;
    let asset_symbol = info.asset_symbol.as_deref();
    let symbol = info.symbol.as_deref();
    print_field("Vault", &info.vault)?;
    print_field("Name", or_unavailable(info.name.as_ref()))?;
    print_field("Symbol", or_unavailable(symbol))?;
    print_field("Decimals", or_unavailable(info.decimals))?;
    print_field("Asset", &info.asset)?;
    print_field("Asset name", or_unavailable(info.asset_name.as_ref()))?;
    print_field("Asset symbol", or_unavailable(asset_symbol))?;
    print_field("Asset decimals", or_unavailable(info.asset_decimals))?;
    print_field("Total assets", display_amount(&info.total_assets, human, asset_symbol))?;
    print_field("Total supply", display_amount(&info.total_supply, human, symbol))?;
    print_field(
        "Assets per share",
        or_unavailable(
            info.assets_per_share.as_ref().map(|a| display_amount(a, human, asset_symbol)),
        ),
    )?;
    print_field(
        "Shares per asset",
        or_unavailable(info.shares_per_asset.as_ref().map(|a| display_amount(a, human, symbol))),
    )
}

fn print_position(position: VaultPosition, human: bool, warnings: Vec<VaultWarning>) -> Result<()> {
    if shell::is_json() {
        return print_json_with_warnings(position, warnings);
    }

    print_warnings(&warnings)?;
    let share_symbol = position.share_symbol.as_deref();
    let asset_symbol = position.asset_symbol.as_deref();
    print_field("Vault", &position.vault)?;
    print_field("Owner", &position.owner)?;
    print_field("Asset", &position.asset)?;
    print_field("Share symbol", or_unavailable(share_symbol))?;
    print_field("Share decimals", or_unavailable(position.share_decimals))?;
    print_field("Asset symbol", or_unavailable(asset_symbol))?;
    print_field("Asset decimals", or_unavailable(position.asset_decimals))?;
    print_field("Share balance", display_amount(&position.share_balance, human, share_symbol))?;
    print_field(
        "Assets equivalent",
        display_amount(&position.assets_equivalent, human, asset_symbol),
    )?;
    print_field("Max withdraw", display_amount(&position.max_withdraw, human, asset_symbol))?;
    print_field("Max redeem", display_amount(&position.max_redeem, human, share_symbol))
}

fn print_compatibility_report(report: &CompatibilityReport) -> Result<()> {
    if shell::is_json() {
        return print_json_success(report);
    }

    print_field("Vault", &report.vault)?;
    print_field("Account", &report.account)?;
    sh_println!("Note: {}", report.disclaimer)?;
    for check in &report.checks {
        let status = match check.status {
            CheckStatus::Pass => "PASS",
            CheckStatus::Warn => "WARN",
            CheckStatus::Fail => "FAIL",
        };
        sh_println!("{status:<4} {:<24} {}", check.name, check.detail)?;
    }
    sh_println!(
        "Summary: {} passed, {} warnings, {} failed",
        report.passed,
        report.warnings,
        report.failed
    )
}

fn print_field(label: &str, value: impl std::fmt::Display) -> Result<()> {
    sh_println!("{label:<20} {value}")
}

fn print_json_with_warnings<T: Serialize>(value: T, warnings: Vec<VaultWarning>) -> Result<()> {
    if warnings.is_empty() {
        print_json_success(value)
    } else {
        print_json_success_with_warnings(
            value,
            warnings
                .into_iter()
                .map(|warning| JsonMessage::warning(warning.code, warning.message))
                .collect(),
        )
    }
}

fn print_warnings(warnings: &[VaultWarning]) -> Result<()> {
    for warning in warnings {
        sh_warn!("{}", warning.message)?;
    }
    Ok(())
}

fn or_unavailable(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "<unavailable>".to_string(), |value| value.to_string())
}

fn display_amount(amount: &TokenAmount, human: bool, symbol: Option<&str>) -> String {
    match (&amount.formatted, symbol.filter(|symbol| !symbol.is_empty())) {
        (Some(formatted), Some(symbol)) if human => format!("{formatted} {symbol}"),
        (Some(formatted), None) if human => formatted.clone(),
        _ => amount.raw.clone(),
    }
}

/// `10^decimals`, or `None` when it overflows.
fn decimal_unit(decimals: u8) -> Option<U256> {
    U256::from(10).checked_pow(U256::from(decimals))
}

fn push_check(
    checks: &mut Vec<CompatibilityCheck>,
    name: impl Into<String>,
    status: CheckStatus,
    detail: impl Into<String>,
) {
    checks.push(CompatibilityCheck { name: name.into(), status, detail: detail.into() });
}

fn record_required<T, E>(
    checks: &mut Vec<CompatibilityCheck>,
    name: &str,
    result: std::result::Result<T, E>,
) {
    let (status, detail) = if result.is_ok() {
        (CheckStatus::Pass, "call succeeded")
    } else {
        (CheckStatus::Fail, "call failed or returned incompatible data")
    };
    push_check(checks, name, status, detail);
}

fn record_zero_conversion<E>(
    checks: &mut Vec<CompatibilityCheck>,
    name: &str,
    result: std::result::Result<U256, E>,
) {
    match result {
        Ok(value) if value.is_zero() => {
            push_check(checks, name, CheckStatus::Pass, "returned zero")
        }
        Ok(value) => push_check(
            checks,
            name,
            CheckStatus::Warn,
            format!("returned {value}; zero input normally converts to zero"),
        ),
        Err(_) => {
            push_check(checks, name, CheckStatus::Fail, "call failed or returned incompatible data")
        }
    }
}

fn record_preview<E>(
    checks: &mut Vec<CompatibilityCheck>,
    name: &str,
    request_kind: &str,
    async_supported: bool,
    result: std::result::Result<U256, E>,
) {
    match result {
        Ok(_) if async_supported => push_check(
            checks,
            name,
            CheckStatus::Warn,
            format!(
                "vault advertises asynchronous ERC-7540 {request_kind} support, which requires \
                 this preview to revert"
            ),
        ),
        Ok(value) if value.is_zero() => {
            push_check(checks, name, CheckStatus::Pass, "returned zero")
        }
        Ok(value) => push_check(
            checks,
            name,
            CheckStatus::Warn,
            format!("returned {value}; a zero-amount preview normally returns zero"),
        ),
        Err(_) if async_supported => push_check(
            checks,
            name,
            CheckStatus::Warn,
            format!(
                "reverted as required by advertised asynchronous ERC-7540 {request_kind} support"
            ),
        ),
        Err(_) => push_check(
            checks,
            name,
            CheckStatus::Fail,
            "call failed or returned incompatible data without advertised ERC-7540 support",
        ),
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
    sh_warn!("{}", zero_exit_warning(method).message)
}

fn warn_if_native_asset(asset: Address) -> Result<()> {
    if asset == NATIVE_ASSET {
        sh_warn!("{}", native_asset_warning().message)?;
    }
    Ok(())
}

fn zero_exit_warning(method: &str) -> VaultWarning {
    VaultWarning {
        code: match method {
            "maxWithdraw" => "erc4626_zero_max_withdraw",
            "maxRedeem" => "erc4626_zero_max_redeem",
            _ => "erc4626_zero_exit_max",
        },
        message: format!(
            "Vault reported zero from {method} even though the owner has shares; liquidity, \
             gates, withdrawal queues, or a conservative implementation may prevent the base \
             ERC-4626 exit."
        ),
    }
}

fn native_asset_warning() -> VaultWarning {
    VaultWarning {
        code: "erc4626_native_asset",
        message: "Vault uses the ERC-7535 native-asset sentinel; base ERC-4626 write commands do \
                  not attach native value, so use `cast send --value` when the vault requires it."
            .to_string(),
    }
}

/// Warns when the vault holds the native asset and resolves the write's account arguments.
async fn prepare_write<const N: usize>(
    vault: &NameOrAddress,
    accounts: [NameOrAddress; N],
    send_tx: &SendTxOpts,
) -> Result<[Address; N]> {
    let (provider, vault) = vault_at(&send_tx.eth.rpc, vault.clone()).await?;
    if let Ok(asset) = vault.asset().call().await {
        warn_if_native_asset(asset)?;
    }

    let mut resolved = [Address::ZERO; N];
    for (slot, account) in resolved.iter_mut().zip(accounts) {
        *slot = account.resolve(&provider).await?;
    }
    Ok(resolved)
}

async fn send_call<C: SolCall>(
    vault: NameOrAddress,
    call: C,
    send_tx: SendTxOpts,
    tx: TxParams,
) -> Result<()> {
    // Boxed to keep the large `cast send` future off this command's stack frame.
    Box::pin(SendTxArgs::contract_call(vault, call.abi_encode(), send_tx, tx).run()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn all_subcommands_document_example_output() {
        let command = Erc4626Subcommand::command();
        let subcommands = command.get_subcommands().collect::<Vec<_>>();
        assert_eq!(subcommands.len(), 19);

        for subcommand in subcommands {
            let help = subcommand
                .get_long_about()
                .unwrap_or_else(|| panic!("{} is missing long help", subcommand.get_name()))
                .to_string();
            assert!(
                help.contains("Example:\n\n```text\n$ cast erc4626"),
                "{} is missing a fenced example command",
                subcommand.get_name()
            );
            assert!(
                help.contains("\n\nOutput:\n\n```text\n"),
                "{} is missing fenced example output",
                subcommand.get_name()
            );
        }

        let info = command.find_subcommand("info").unwrap().get_long_about().unwrap().to_string();
        assert!(info.contains("--human"));
    }
}
