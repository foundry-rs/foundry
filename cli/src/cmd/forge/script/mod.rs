mod build;
use build::BuildOutput;

mod runner;
use runner::Runner;

mod broadcast;
use broadcast::into_legacy;

mod cmd;

mod executor;

use crate::{cmd::forge::build::BuildArgs, opts::MultiWallet};
use clap::{Parser, ValueHint};
use ethers::{
    abi::RawLog,
    types::{transaction::eip2718::TypedTransaction, Address},
};
use forge::{
    debug::DebugArena,
    trace::{CallTraceArena, TraceKind},
};
use foundry_common::evm::EvmArgs;

use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(ScriptArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct ScriptArgs {
    /// The path of the contract to run.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[clap(value_hint = ValueHint::FilePath)]
    pub path: PathBuf,

    /// Arguments to pass to the script function.
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[clap(long, alias = "tc")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[clap(long, short, default_value = "run()")]
    pub sig: String,

    #[clap(
        long,
        help = "Use legacy transactions instead of EIP1559 ones. this is auto-enabled for common networks without EIP1559."
    )]
    pub legacy: bool,

    #[clap(long, help = "Execute the transactions.")]
    pub execute: bool,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    pub opts: BuildArgs,

    #[clap(flatten)]
    pub wallets: MultiWallet,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,

    /// Resumes submitting transactions that failed or timed-out previously.
    ///
    /// It DOES NOT simulate the script again and it expects nonces to have remained the same.
    ///
    /// Example: If transaction N has a nonce of 22, then the account should have a nonce of 22,
    /// otherwise it fails.
    #[clap(long)]
    pub resume: bool,

    /// Resumes submitting transactions that failed or timed-out previously. SHOULD NOT be used
    /// when deploying CREATE contracts.
    ///
    /// It DOES NOT simulate the script again and IT DOES NOT expect nonces to have remained the
    /// same.
    ///
    /// Example: If transaction N has a nonce of 22 and the account has a nonce of 23, then
    /// all transactions from this account will have a nonce of TX_NONCE + 1.
    #[clap(long)]
    pub force_resume: bool,

    #[clap(long, help = "Address which will deploy all library dependencies.")]
    pub deployer: Option<Address>,
}

pub struct ScriptResult {
    pub success: bool,
    pub logs: Vec<RawLog>,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: Option<Vec<DebugArena>>,
    pub gas: u64,
    pub labeled_addresses: BTreeMap<Address, String>,
    pub transactions: Option<VecDeque<TypedTransaction>>,
}
