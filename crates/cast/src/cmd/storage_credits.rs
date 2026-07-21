use crate::{
    cmd::{
        keychain::ensure_tempo_precompile_active,
        tip20::{resolve_tip20_signer, send_tip20_transaction},
    },
    tempo::print_payload,
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use clap::{Parser, ValueEnum};
use eyre::Result;
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::provider::ProviderBuilder;
use foundry_evm::hardfork::TempoHardfork;
use serde_json::json;
use std::str::FromStr;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{IStorageCredits, STORAGE_CREDITS_ADDRESS};

/// T7 storage credits operations (Tempo).
///
/// Storage credits are a per-account, non-transferable balance minted when an account frees its own
/// storage and later spent to discount the creation cost of new storage. This wraps the T7
/// StorageCredits precompile at `0x1060000000000000000000000000000000000000`.
#[derive(Debug, Parser, Clone)]
pub enum StorageCreditsSubcommand {
    /// Show an account's storage credit balance.
    Balance {
        /// Account to query.
        #[arg(value_parser = NameOrAddress::from_str)]
        account: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Show an account's storage credit consumption mode.
    ///
    /// Mode is transaction-local transient state, so a standalone read reflects the default rather
    /// than a value set by an earlier `set-mode` transaction.
    Mode {
        /// Account to query.
        #[arg(value_parser = NameOrAddress::from_str)]
        account: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Show an account's storage credit spend budget.
    ///
    /// Budget is transaction-local transient state, so a standalone read reflects the default
    /// rather than a value set by an earlier `set-budget` transaction.
    Budget {
        /// Account to query.
        #[arg(value_parser = NameOrAddress::from_str)]
        account: NameOrAddress,

        #[command(flatten)]
        rpc: RpcOpts,
    },

    /// Set the caller's storage credit consumption mode.
    ///
    /// The mode only applies within the transaction that sets it; batch it with the storage
    /// operations it should govern.
    SetMode {
        /// Mode to switch to.
        #[arg(value_enum)]
        mode: CreditMode,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },

    /// Set the caller's storage credit spend budget, which also selects `direct` mode.
    ///
    /// The budget only applies within the transaction that sets it; batch it with the storage
    /// operations it should govern.
    SetBudget {
        /// Maximum number of credits the caller may spend in `direct` mode this transaction.
        credits: u64,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: TxParams,
    },
}

/// CLI-facing spelling of `IStorageCredits::Mode`.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CreditMode {
    /// Pay creation cost upfront, then settle credits as a refund at end of transaction.
    Refund,
    /// Pay creation cost upfront and keep freed credits instead of spending them.
    Preserve,
    /// Spend existing credits synchronously; selecting this sets an effectively unlimited budget.
    Direct,
}

impl StorageCreditsSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Balance { account, rpc } => balance(account, rpc).await,
            Self::Mode { account, rpc } => mode(account, rpc).await,
            Self::Budget { account, rpc } => budget(account, rpc).await,
            Self::SetMode { mode, send_tx, tx } => set_mode(mode, send_tx, tx).await,
            Self::SetBudget { credits, send_tx, tx } => set_budget(credits, send_tx, tx).await,
        }
    }
}

async fn balance(account: NameOrAddress, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    ensure_storage_credits_t7(&provider, "cast storage-credits balance").await?;
    let account = account.resolve(&provider).await?;

    let credits = IStorageCredits::new(STORAGE_CREDITS_ADDRESS, &provider);
    let balance = credits.balanceOf(account).call().await?;
    let payload = json!({ "account": format!("{account}"), "balance": balance });
    print_payload(payload, |payload| {
        sh_println!(
            "Account: {}\nBalance: {}",
            payload["account"].as_str().unwrap_or_default(),
            payload["balance"],
        )
    })
}

async fn mode(account: NameOrAddress, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    ensure_storage_credits_t7(&provider, "cast storage-credits mode").await?;
    let account = account.resolve(&provider).await?;

    let credits = IStorageCredits::new(STORAGE_CREDITS_ADDRESS, &provider);
    let mode = credits.modeOf(account).call().await?;
    let payload = json!({ "account": format!("{account}"), "mode": mode.as_str() });
    print_payload(payload, |payload| {
        sh_println!(
            "Account: {}\nMode:    {}",
            payload["account"].as_str().unwrap_or_default(),
            payload["mode"].as_str().unwrap_or_default(),
        )
    })
}

async fn budget(account: NameOrAddress, rpc: RpcOpts) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    ensure_storage_credits_t7(&provider, "cast storage-credits budget").await?;
    let account = account.resolve(&provider).await?;

    let credits = IStorageCredits::new(STORAGE_CREDITS_ADDRESS, &provider);
    let budget = credits.budgetOf(account).call().await?;
    let payload = json!({ "account": format!("{account}"), "budget": budget });
    print_payload(payload, |payload| {
        sh_println!(
            "Account: {}\nBudget:  {}",
            payload["account"].as_str().unwrap_or_default(),
            payload["budget"],
        )
    })
}

async fn set_mode(mode: CreditMode, send_tx: SendTxOpts, tx: TxParams) -> Result<()> {
    ensure_send_storage_credits_t7(&send_tx, "cast storage-credits set-mode").await?;
    let (signer, access_key) = resolve_tip20_signer(&send_tx, &tx).await?;
    // The precompile ABI encodes `Mode` as its `uint8` discriminant.
    let mode_arg = (mode.to_sol() as u8).to_string();
    send_tip20_transaction(
        NameOrAddress::Address(STORAGE_CREDITS_ADDRESS),
        "setMode(uint8)",
        vec![mode_arg],
        send_tx,
        tx,
        signer,
        access_key,
    )
    .await
}

async fn set_budget(credits: u64, send_tx: SendTxOpts, tx: TxParams) -> Result<()> {
    ensure_send_storage_credits_t7(&send_tx, "cast storage-credits set-budget").await?;
    let (signer, access_key) = resolve_tip20_signer(&send_tx, &tx).await?;
    send_tip20_transaction(
        NameOrAddress::Address(STORAGE_CREDITS_ADDRESS),
        "setBudget(uint64)",
        vec![credits.to_string()],
        send_tx,
        tx,
        signer,
        access_key,
    )
    .await
}

/// The StorageCredits precompile only exists on T7+; fail early with a clear message instead of
/// surfacing a raw revert. Fall back to a code check when the RPC lacks the hardfork query.
async fn ensure_storage_credits_t7<P>(provider: &P, command: &str) -> Result<()>
where
    P: alloy_provider::Provider<TempoNetwork>,
{
    ensure_tempo_precompile_active(
        provider,
        TempoHardfork::T7,
        STORAGE_CREDITS_ADDRESS,
        &format!("{command} requires a Tempo T7-capable StorageCredits RPC"),
    )
    .await
}

/// Gate a write command on T7 before signing: on pre-T7 the precompile address is an empty account,
/// so a transaction to it would silently succeed as a no-op.
async fn ensure_send_storage_credits_t7(send_tx: &SendTxOpts, command: &str) -> Result<()> {
    let config = send_tx.eth.rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
    ensure_storage_credits_t7(&provider, command).await
}

impl CreditMode {
    const fn to_sol(self) -> IStorageCredits::Mode {
        match self {
            Self::Refund => IStorageCredits::Mode::Refund,
            Self::Preserve => IStorageCredits::Mode::Preserve,
            Self::Direct => IStorageCredits::Mode::Direct,
        }
    }
}
