use crate::{
    cmd::tip20::send_tip20_transaction,
    tempo::{ensure_tempo_precompile_active, print_payload, tempo_provider},
    tx::{SendTxOpts, TxParams},
};
use alloy_ens::NameOrAddress;
use alloy_primitives::Address;
use alloy_sol_types::SolCall;
use clap::{Parser, ValueEnum};
use eyre::Result;
use foundry_cli::opts::RpcOpts;
use foundry_common::provider::RetryProvider;
use foundry_evm::hardfork::TempoHardfork;
use serde_json::{Value, json};
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
            Self::Balance { account, rpc } => {
                read(account, rpc, "balance", "Balance:", |credits, account| async move {
                    Ok(json!(credits.balanceOf(account).call().await?))
                })
                .await
            }
            Self::Mode { account, rpc } => {
                read(account, rpc, "mode", "Mode:   ", |credits, account| async move {
                    Ok(json!(credits.modeOf(account).call().await?.as_str()))
                })
                .await
            }
            Self::Budget { account, rpc } => {
                read(account, rpc, "budget", "Budget: ", |credits, account| async move {
                    Ok(json!(credits.budgetOf(account).call().await?))
                })
                .await
            }
            Self::SetMode { mode, send_tx, tx } => {
                ensure_t7(&send_tx.eth.rpc, "cast storage-credits set-mode").await?;
                let new_mode = match mode {
                    CreditMode::Refund => IStorageCredits::Mode::Refund,
                    CreditMode::Preserve => IStorageCredits::Mode::Preserve,
                    CreditMode::Direct => IStorageCredits::Mode::Direct,
                };
                let data = IStorageCredits::setModeCall { newMode: new_mode }.abi_encode();
                send_tip20_transaction(STORAGE_CREDITS_ADDRESS, data, send_tx, tx).await
            }
            Self::SetBudget { credits, send_tx, tx } => {
                ensure_t7(&send_tx.eth.rpc, "cast storage-credits set-budget").await?;
                let data = IStorageCredits::setBudgetCall { credits }.abi_encode();
                send_tip20_transaction(STORAGE_CREDITS_ADDRESS, data, send_tx, tx).await
            }
        }
    }
}

type Credits = IStorageCredits::IStorageCreditsInstance<RetryProvider<TempoNetwork>, TempoNetwork>;

/// Reads one account field from the precompile and prints it as `key` in JSON mode and after
/// `label` otherwise.
async fn read<F, Fut>(
    account: NameOrAddress,
    rpc: RpcOpts,
    key: &str,
    label: &str,
    query: F,
) -> Result<()>
where
    F: FnOnce(Credits, Address) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let provider = ensure_t7(&rpc, &format!("cast storage-credits {key}")).await?;
    let account = account.resolve(&provider).await?;
    let value = query(IStorageCredits::new(STORAGE_CREDITS_ADDRESS, provider), account).await?;
    let payload = json!({ "account": format!("{account}"), key: value });
    print_payload(payload, |payload| {
        let value = &payload[key];
        let value = value.as_str().map_or_else(|| value.to_string(), str::to_string);
        sh_println!("Account: {}\n{label} {value}", payload["account"].as_str().unwrap_or_default())
    })
}

/// The StorageCredits precompile only exists on T7+; fail early with a clear message instead of
/// surfacing a raw revert (or, for writes, a silently successful no-op transaction to an empty
/// account).
async fn ensure_t7(rpc: &RpcOpts, command: &str) -> Result<RetryProvider<TempoNetwork>> {
    let (_, provider) = tempo_provider(rpc)?;
    ensure_tempo_precompile_active(
        &provider,
        TempoHardfork::T7,
        STORAGE_CREDITS_ADDRESS,
        &format!("{command} requires a Tempo T7-capable StorageCredits RPC"),
    )
    .await?;
    Ok(provider)
}
