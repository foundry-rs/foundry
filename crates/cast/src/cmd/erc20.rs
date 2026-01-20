use std::str::FromStr;

use crate::{
    cmd::send::cast_send,
    format_uint_exp,
    tx::{CastTxSender, SendTxOpts, signing_provider_with_curl},
};
use alloy_eips::{BlockId, Encodable2718};
use alloy_ens::NameOrAddress;
use alloy_network::{AnyNetwork, NetworkWallet, TransactionBuilder};
use alloy_primitives::{U64, U256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_sol_types::sol;
use clap::{Args, Parser};
use foundry_cli::{
    opts::{RpcOpts, TempoOpts},
    utils::{LoadConfig, get_chain, get_provider_with_curl},
};
#[doc(hidden)]
pub use foundry_config::{Chain, utils::*};
use foundry_primitives::FoundryTransactionRequest;

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

/// Apply transaction options to a transaction request for ERC20 operations.
fn apply_tx_opts(
    tx: &mut WithOtherFields<TransactionRequest>,
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
        tx.other.insert("feeToken".to_string(), serde_json::to_value(fee_token).unwrap());
    }

    if let Some(nonce_key) = tx_opts.tempo.sequence_key {
        tx.other.insert("nonceKey".to_string(), serde_json::to_value(nonce_key).unwrap());
    }
}

/// Send an ERC20 transaction, handling Tempo transactions specially if needed
///
/// TODO: Remove this temporary helper when we migrate to FoundryNetwork/FoundryTransactionRequest.
async fn send_erc20_tx<P: Provider<AnyNetwork>>(
    provider: P,
    tx: WithOtherFields<TransactionRequest>,
    send_tx: &SendTxOpts,
    timeout: u64,
) -> eyre::Result<()> {
    // Same as in SendTxArgs::run(), Tempo transactions need to be signed locally and sent as raw
    // transactions
    if tx.other.contains_key("feeToken") || tx.other.contains_key("nonceKey") {
        let signer = send_tx.eth.wallet.signer().await?;
        let mut ftx = FoundryTransactionRequest::new(tx);
        if ftx.chain_id().is_none() {
            ftx.set_chain_id(provider.get_chain_id().await?);
        }

        // Sign using NetworkWallet<FoundryNetwork>
        let signed_tx = signer.sign_request(ftx).await?;

        // Encode and send raw
        let mut raw_tx = Vec::with_capacity(signed_tx.encode_2718_len());
        signed_tx.encode_2718(&mut raw_tx);

        let cast = CastTxSender::new(&provider);
        let pending_tx = cast.send_raw(&raw_tx).await?;
        let tx_hash = pending_tx.inner().tx_hash();

        if send_tx.cast_async {
            sh_println!("{tx_hash:#x}")?;
        } else {
            // For sync mode, we already have the hash, just wait for receipt
            let receipt = cast
                .receipt(format!("{tx_hash:#x}"), None, send_tx.confirmations, Some(timeout), false)
                .await?;
            sh_println!("{receipt}")?;
        }

        return Ok(());
    }

    // Use the normal cast_send path for non-Tempo transactions
    cast_send(provider, tx, send_tx.cast_async, send_tx.sync, send_tx.confirmations, timeout).await
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
            Self::Allowance { token, owner, spender, block, rpc, .. } => {
                let provider = get_provider_with_curl(&config, rpc.curl)?;
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
            Self::Balance { token, owner, block, rpc, .. } => {
                let provider = get_provider_with_curl(&config, rpc.curl)?;
                let token = token.resolve(&provider).await?;
                let owner = owner.resolve(&provider).await?;

                let balance = IERC20::new(token, &provider)
                    .balanceOf(owner)
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", format_uint_exp(balance))?
            }
            Self::Name { token, block, rpc, .. } => {
                let provider = get_provider_with_curl(&config, rpc.curl)?;
                let token = token.resolve(&provider).await?;

                let name = IERC20::new(token, &provider)
                    .name()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", name)?
            }
            Self::Symbol { token, block, rpc, .. } => {
                let provider = get_provider_with_curl(&config, rpc.curl)?;
                let token = token.resolve(&provider).await?;

                let symbol = IERC20::new(token, &provider)
                    .symbol()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", symbol)?
            }
            Self::Decimals { token, block, rpc, .. } => {
                let provider = get_provider_with_curl(&config, rpc.curl)?;
                let token = token.resolve(&provider).await?;

                let decimals = IERC20::new(token, &provider)
                    .decimals()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", decimals)?
            }
            Self::TotalSupply { token, block, rpc, .. } => {
                let provider = get_provider_with_curl(&config, rpc.curl)?;
                let token = token.resolve(&provider).await?;

                let total_supply = IERC20::new(token, &provider)
                    .totalSupply()
                    .block(block.unwrap_or_default())
                    .call()
                    .await?;
                sh_println!("{}", format_uint_exp(total_supply))?
            }
            // State-changing
            Self::Transfer { token, to, amount, send_tx, tx: tx_opts, .. } => {
                let provider = signing_provider_with_curl(&send_tx, send_tx.eth.rpc.curl).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .transfer(to.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                send_erc20_tx(
                    provider,
                    tx,
                    &send_tx,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
            Self::Approve { token, spender, amount, send_tx, tx: tx_opts, .. } => {
                let provider = signing_provider_with_curl(&send_tx, send_tx.eth.rpc.curl).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .approve(spender.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                send_erc20_tx(
                    provider,
                    tx,
                    &send_tx,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
            Self::Mint { token, to, amount, send_tx, tx: tx_opts, .. } => {
                let provider = signing_provider_with_curl(&send_tx, send_tx.eth.rpc.curl).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .mint(to.resolve(&provider).await?, U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                send_erc20_tx(
                    provider,
                    tx,
                    &send_tx,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
            Self::Burn { token, amount, send_tx, tx: tx_opts, .. } => {
                let provider = signing_provider_with_curl(&send_tx, send_tx.eth.rpc.curl).await?;
                let mut tx = IERC20::new(token.resolve(&provider).await?, &provider)
                    .burn(U256::from_str(&amount)?)
                    .into_transaction_request();

                // Apply transaction options using helper
                apply_tx_opts(
                    &mut tx,
                    &tx_opts,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                send_erc20_tx(
                    provider,
                    tx,
                    &send_tx,
                    send_tx.timeout.unwrap_or(config.transaction_timeout),
                )
                .await?
            }
        };
        Ok(())
    }
}
