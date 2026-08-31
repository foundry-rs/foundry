use alloy_primitives::{Address, B256, Bytes, U256};
use clap::{Parser, ValueEnum};
use eyre::Result;
use foundry_cli::{
    opts::{RpcOpts, TransactionOpts},
    utils::parse_ether_value,
};
use foundry_wallets::WalletOpts;

mod contracts;
mod delegates;
mod deploy;
mod execute;
mod proposal;
mod service;
mod signing;
mod simulate;
mod transaction;

use contracts::{
    COMPATIBILITY_FALLBACK_HANDLER_V1_4_1, SAFE_PROXY_FACTORY_V1_4_1, SIMULATE_TX_ACCESSOR_V1_4_1,
};
pub use service::SafeServiceOpts;

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
    Create {
        /// Addresses that own the Safe.
        #[arg(required = true, num_args = 1..)]
        owners: Vec<Address>,

        /// Number of owner signatures required. Defaults to all owners.
        #[arg(long)]
        threshold: Option<usize>,

        /// CREATE2 salt nonce. Defaults to Safe Protocol Kit's chain-specific nonce.
        #[arg(long)]
        salt_nonce: Option<U256>,

        /// Safe singleton address. Defaults to the canonical v1.4.1 deployment.
        #[arg(long, conflicts_with = "l1")]
        singleton: Option<Address>,

        /// Use the L1 Safe singleton instead of SafeL2.
        #[arg(long)]
        l1: bool,

        /// SafeProxyFactory address.
        #[arg(long, default_value_t = SAFE_PROXY_FACTORY_V1_4_1)]
        factory: Address,

        /// CompatibilityFallbackHandler address. Pass the zero address to disable it.
        #[arg(long, default_value_t = COMPATIBILITY_FALLBACK_HANDLER_V1_4_1)]
        fallback_handler: Address,

        /// Number of confirmations to wait for.
        #[arg(long, default_value = "1")]
        confirmations: u64,

        /// Timeout for deployment confirmation, in seconds.
        #[arg(long, env = "ETH_TIMEOUT")]
        timeout: Option<u64>,

        /// Polling interval for the deployment receipt, in seconds.
        #[arg(long, env = "ETH_POLL_INTERVAL")]
        poll_interval: Option<u64>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,

        #[command(flatten)]
        tx: Box<TransactionOpts>,
    },

    /// Register a transaction-service delegate for a Safe owner.
    AddDelegate {
        /// Safe account address.
        safe: Address,

        /// Address allowed to propose transactions.
        delegate: Address,

        /// Human-readable delegate label.
        #[arg(long)]
        label: String,

        #[command(flatten)]
        service: Box<SafeServiceOpts>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,
    },

    /// List transaction-service delegates registered for a Safe.
    ListDelegates {
        /// Safe account address.
        safe: Address,

        #[command(flatten)]
        service: Box<SafeServiceOpts>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,
    },

    /// Remove a transaction-service delegate for a Safe owner.
    RemoveDelegate {
        /// Safe account address.
        safe: Address,

        /// Delegate address to remove.
        delegate: Address,

        #[command(flatten)]
        service: Box<SafeServiceOpts>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,
    },

    /// Create, sign, and submit a Safe transaction proposal.
    Propose {
        /// Safe account address.
        safe: Address,

        /// Transaction target.
        to: Address,

        /// Function signature to call.
        sig: Option<String>,

        /// Function arguments.
        #[arg(allow_negative_numbers = true)]
        args: Vec<String>,

        /// Raw calldata. Cannot be combined with a function signature or arguments.
        #[arg(long, conflicts_with_all = ["sig", "args"])]
        data: Option<Bytes>,

        /// Native token value sent by the Safe.
        #[arg(long, default_value = "0", value_parser = parse_ether_value)]
        value: U256,

        /// Safe operation type.
        #[arg(long, value_enum, default_value_t = SafeOperation::Call)]
        operation: SafeOperation,

        /// Safe transaction gas.
        #[arg(long, default_value = "0")]
        safe_tx_gas: U256,

        /// Base gas reimbursed by the Safe.
        #[arg(long, default_value = "0")]
        base_gas: U256,

        /// Gas price reimbursed by the Safe.
        #[arg(long, default_value = "0")]
        gas_price: U256,

        /// Token used for gas reimbursement.
        #[arg(long, default_value_t = Address::ZERO)]
        gas_token: Address,

        /// Gas reimbursement receiver.
        #[arg(long, default_value_t = Address::ZERO)]
        refund_receiver: Address,

        /// Safe nonce. Defaults to the next queued Transaction Service nonce.
        #[arg(long)]
        nonce: Option<U256>,

        /// Optional origin shown by Safe clients.
        #[arg(long)]
        origin: Option<String>,

        #[command(flatten)]
        service: Box<SafeServiceOpts>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,
    },

    /// Sign and submit a confirmation for a proposed Safe transaction.
    Sign {
        /// Safe account address.
        safe: Address,

        /// Safe transaction hash from the Transaction Service.
        safe_tx_hash: B256,

        #[command(flatten)]
        service: Box<SafeServiceOpts>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,
    },

    /// Simulate a proposed Safe transaction without requiring owner signatures.
    ///
    /// This simulates the inner CALL or DELEGATECALL in the Safe's context. It does not validate
    /// the Safe nonce, owner signatures, threshold, or guard hooks. Reimbursed transactions
    /// (`gasPrice > 0`) are rejected because SimulateTxAccessor does not enforce `safeTxGas`.
    #[command(verbatim_doc_comment)]
    Simulate {
        /// Safe account address.
        safe: Address,

        /// Safe transaction hash from the Transaction Service.
        safe_tx_hash: B256,

        /// Address that will execute the Safe transaction. Used as the simulation's tx.origin.
        #[arg(long, env = "ETH_FROM", value_name = "ADDRESS")]
        from: Address,

        /// SimulateTxAccessor address.
        #[arg(long, default_value_t = SIMULATE_TX_ACCESSOR_V1_4_1)]
        accessor: Address,

        #[command(flatten)]
        service: Box<SafeServiceOpts>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,
    },

    /// Execute a confirmed Safe transaction onchain.
    Execute {
        /// Safe account address.
        safe: Address,

        /// Safe transaction hash from the Transaction Service.
        safe_tx_hash: B256,

        /// Number of confirmations to wait for.
        #[arg(long, default_value = "1")]
        confirmations: u64,

        /// Timeout for execution confirmation, in seconds.
        #[arg(long, env = "ETH_TIMEOUT")]
        timeout: Option<u64>,

        /// Polling interval for the execution receipt, in seconds.
        #[arg(long, env = "ETH_POLL_INTERVAL")]
        poll_interval: Option<u64>,

        #[command(flatten)]
        service: Box<SafeServiceOpts>,

        #[command(flatten)]
        rpc: Box<RpcOpts>,

        #[command(flatten)]
        wallet: Box<WalletOpts>,

        #[command(flatten)]
        tx: Box<TransactionOpts>,
    },
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
            Self::Create {
                owners,
                threshold,
                salt_nonce,
                singleton,
                l1,
                factory,
                fallback_handler,
                confirmations,
                timeout,
                poll_interval,
                rpc,
                wallet,
                tx,
            } => {
                deploy::run(
                    owners,
                    threshold,
                    salt_nonce,
                    singleton,
                    l1,
                    factory,
                    fallback_handler,
                    confirmations,
                    timeout,
                    poll_interval,
                    *rpc,
                    *wallet,
                    *tx,
                )
                .await?;
            }
            Self::AddDelegate { safe, delegate, label, service, rpc, wallet } => {
                delegates::add(safe, delegate, label, *service, *rpc, *wallet).await?;
            }
            Self::ListDelegates { safe, service, rpc } => {
                delegates::list(safe, *service, *rpc).await?;
            }
            Self::RemoveDelegate { safe, delegate, service, rpc, wallet } => {
                delegates::remove(safe, delegate, *service, *rpc, *wallet).await?;
            }
            Self::Propose {
                safe,
                to,
                sig,
                args,
                data,
                value,
                operation,
                safe_tx_gas,
                base_gas,
                gas_price,
                gas_token,
                refund_receiver,
                nonce,
                origin,
                service,
                rpc,
                wallet,
            } => {
                proposal::propose(
                    safe,
                    to,
                    sig,
                    args,
                    data,
                    value,
                    operation,
                    safe_tx_gas,
                    base_gas,
                    gas_price,
                    gas_token,
                    refund_receiver,
                    nonce,
                    origin,
                    *service,
                    *rpc,
                    *wallet,
                )
                .await?;
            }
            Self::Sign { safe, safe_tx_hash, service, rpc, wallet } => {
                proposal::sign(safe, safe_tx_hash, *service, *rpc, *wallet).await?;
            }
            Self::Simulate { safe, safe_tx_hash, from, accessor, service, rpc } => {
                simulate::run(safe, safe_tx_hash, from, accessor, *service, *rpc).await?;
            }
            Self::Execute {
                safe,
                safe_tx_hash,
                confirmations,
                timeout,
                poll_interval,
                service,
                rpc,
                wallet,
                tx,
            } => {
                execute::run(
                    safe,
                    safe_tx_hash,
                    confirmations,
                    timeout,
                    poll_interval,
                    *service,
                    *rpc,
                    *wallet,
                    *tx,
                )
                .await?;
            }
        }
        Ok(())
    }
}
