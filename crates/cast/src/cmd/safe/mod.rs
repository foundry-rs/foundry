use alloy_network::Ethereum;
use alloy_provider::Provider;
use clap::{Parser, ValueEnum};
use eyre::Result;
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::provider::{ProviderBuilder, RetryProvider};

mod contracts;
mod delegates;
mod deploy;
mod execute;
mod proposal;
mod service;
mod signing;
mod simulate;
mod transaction;

/// Safe transaction operations.
#[derive(Debug, Parser)]
pub enum SafeSubcommand {
    /// Deploy a Safe account.
    ///
    /// Examples:
    /// - cast safe create $OWNER --threshold 1 --rpc-url $RPC --ledger
    /// - cast safe create $OWNER_1 $OWNER_2 $OWNER_3 --threshold 2 --rpc-url $RPC --account
    ///   deployer
    #[command(verbatim_doc_comment)]
    Create(deploy::CreateArgs),

    /// Register a transaction-service delegate for a Safe owner.
    AddDelegate(delegates::AddDelegateArgs),

    /// List transaction-service delegates registered for a Safe.
    ListDelegates(delegates::ListDelegatesArgs),

    /// Remove a transaction-service delegate for a Safe owner.
    RemoveDelegate(delegates::RemoveDelegateArgs),

    /// Create, sign, and submit a Safe transaction proposal.
    Propose(proposal::ProposeArgs),

    /// Sign and submit a confirmation for a proposed Safe transaction.
    Sign(proposal::SignArgs),

    /// Simulate a proposed Safe transaction without requiring owner signatures.
    ///
    /// This simulates the inner CALL or DELEGATECALL in the Safe's context. It does not validate
    /// the Safe nonce, owner signatures, threshold, or guard hooks. Reimbursed transactions
    /// (`gasPrice > 0`) are rejected because SimulateTxAccessor does not enforce `safeTxGas`.
    #[command(verbatim_doc_comment)]
    Simulate(simulate::SimulateArgs),

    /// Execute a confirmed Safe transaction onchain.
    Execute(execute::ExecuteArgs),
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[repr(u8)]
pub enum SafeOperation {
    #[default]
    Call = 0,
    DelegateCall = 1,
}

impl SafeSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Create(args) => args.run().await,
            Self::AddDelegate(args) => args.run().await,
            Self::ListDelegates(args) => args.run().await,
            Self::RemoveDelegate(args) => args.run().await,
            Self::Propose(args) => args.run().await,
            Self::Sign(args) => args.run().await,
            Self::Simulate(args) => args.run().await,
            Self::Execute(args) => args.run().await,
        }
    }
}

/// Builds the read-only provider for `rpc` and returns it together with its chain ID.
async fn rpc_provider(rpc: &RpcOpts) -> Result<(RetryProvider<Ethereum>, u64)> {
    let provider = ProviderBuilder::<Ethereum>::from_config(&rpc.load_config()?)?.build()?;
    let chain_id = provider.get_chain_id().await?;
    Ok((provider, chain_id))
}
