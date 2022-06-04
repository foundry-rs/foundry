use super::{build::BuildArgs, script::ScriptArgs, watch::WatchArgs};
use crate::{cmd::forge::build::CoreBuildArgs, opts::MultiWallet};
use clap::{Parser, ValueHint};
use foundry_common::evm::EvmArgs;
use std::path::PathBuf;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(DebugArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct DebugArgs {
    /// The contract you want to run. Either the file path or contract name.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[clap(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub path: PathBuf,

    /// Arguments to pass to the script function.
    #[clap(value_name = "ARGS")]
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[clap(long, visible_alias = "tc", value_name = "CONTRACT_NAME")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[clap(long, short, default_value = "run()", value_name = "SIGNATURE")]
    pub sig: String,

    /// Open the script in the debugger.
    #[clap(long)]
    pub debug: bool,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    pub opts: CoreBuildArgs,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,
}

impl DebugArgs {
    pub async fn debug(self) -> eyre::Result<()> {
        let script = ScriptArgs {
            path: self.path.to_str().expect("Invalid path string.").to_string(),
            args: self.args,
            target_contract: self.target_contract,
            sig: self.sig,
            legacy: false,
            broadcast: false,
            opts: BuildArgs {
                args: self.opts,
                names: false,
                sizes: false,
                watch: WatchArgs::default(),
            },
            wallets: MultiWallet::default(),
            evm_opts: self.evm_opts,
            resume: false,
            debug: true,
            slow: false,
            etherscan_api_key: None,
            verify: false,
            json: false,
            with_gas_price: None,
        };
        script.run_script().await
    }
}
