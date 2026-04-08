use std::{str::FromStr, time::Duration};

use crate::{cmd::send::cast_send, format_uint_exp, tx::SendTxOpts};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::BlockId;
use alloy_ens::NameOrAddress;
use alloy_network::{AnyNetwork, EthereumWallet, Network, TransactionBuilder};
use alloy_primitives::{Address, U64, U256};
use alloy_provider::{Provider, fillers::RecommendedFillers};
use alloy_signer::Signature;
use alloy_sol_types::sol;
use clap::{Args, Parser};
use foundry_cli::{
    opts::{RpcOpts, TempoOpts},
    utils::{LoadConfig, get_chain, get_provider},
};
use foundry_common::{
    FoundryTransactionBuilder,
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::{ProviderBuilder, RetryProviderWithSigner},
    shell,
};
#[doc(hidden)]
pub use foundry_config::{Chain, utils::*};
use foundry_wallets::{TempoAccessKeyConfig, WalletSigner};
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};

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
#[command(next_help_heading = "Transaction options")]
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

impl Erc20TxOpts {
    /// Applies gas, fee, nonce, and Tempo options to a transaction request.
    fn apply<N: Network>(&self, tx: &mut N::TransactionRequest, legacy: bool)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        if let Some(gas_limit) = self.gas_limit {
            tx.set_gas_limit(gas_limit.to());
        }

        if let Some(gas_price) = self.gas_price {
            if legacy {
                tx.set_gas_price(gas_price.to());
            } else {
                tx.set_max_fee_per_gas(gas_price.to());
            }
        }

        if !legacy && let Some(priority_fee) = self.priority_gas_price {
            tx.set_max_priority_fee_per_gas(priority_fee.to());
        }

        self.tempo.apply::<N>(tx, self.nonce.map(|n| n.to()));
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

        let is_tempo = self.erc20_opts().is_some_and(|erc20| erc20.tempo.is_tempo())
            || tempo_access_key.is_some();

        if is_tempo {
            self.run_generic::<TempoNetwork>(signer, tempo_access_key).await
        } else {
            self.run_generic::<AnyNetwork>(signer, None).await
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
                    let signer =
                        pre_resolved_signer.as_ref().expect("signer required for access key");
                    let $provider =
                        ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
                    let $erc20 = IERC20::new($token.resolve(&$provider).await?, &$provider);
                    let mut tx = { $build_tx }.into_transaction_request();
                    $tx_opts.apply::<TempoNetwork>(
                        &mut tx,
                        get_chain(config.chain, &$provider).await?.is_legacy(),
                    );
                    apply_tempo_access_key::<TempoNetwork>(&mut tx, Some(access_key));
                    send_tempo_keychain(
                        &$provider,
                        tx,
                        signer,
                        access_key,
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

/// Applies Tempo access key fields (from, key_id) to a transaction request.
///
/// Note: `key_authorization` is intentionally not set here. It is only included
/// if the key is not yet provisioned on-chain (checked in [`send_tempo_keychain`]).
fn apply_tempo_access_key<N: Network>(
    tx: &mut N::TransactionRequest,
    config: Option<&TempoAccessKeyConfig>,
) where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if let Some(config) = config {
        tx.set_from(config.wallet_address);
        tx.set_key_id(config.key_address);
    }
}

/// Sends a Tempo transaction using access key (keychain V2 mode).
///
/// Signs the transaction with the access key and sends it via `send_raw_transaction`,
/// bypassing `EthereumWallet`. Only includes `key_authorization` if the key is not yet
/// provisioned on-chain.
async fn send_tempo_keychain<P: Provider<TempoNetwork>>(
    provider: &P,
    mut tx: <TempoNetwork as Network>::TransactionRequest,
    signer: &WalletSigner,
    access_key: &TempoAccessKeyConfig,
    cast_async: bool,
    confirmations: u64,
    timeout: u64,
) -> eyre::Result<()> {
    // Only include key_authorization if the key is not yet provisioned on-chain.
    if let Some(ref auth) = access_key.key_authorization
        && !is_key_provisioned(provider, access_key.wallet_address, access_key.key_address).await
    {
        tx.set_key_authorization(auth.clone());
    }

    let raw_tx = tx.sign_with_access_key(signer, access_key.wallet_address).await?;

    let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();

    let cast = crate::tx::CastTxSender::new(provider);
    cast.print_tx_result(tx_hash, cast_async, confirmations, timeout).await
}

/// Checks whether an access key is already provisioned on-chain.
async fn is_key_provisioned<P: Provider<TempoNetwork>>(
    provider: &P,
    wallet_address: Address,
    key_address: Address,
) -> bool {
    match provider.get_keychain_key(wallet_address, key_address).await {
        Ok(info) => info.keyId != Address::ZERO,
        Err(_) => false,
    }
}
