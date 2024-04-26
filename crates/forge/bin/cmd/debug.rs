use clap::{Parser, ValueHint};
use forge_script::ScriptArgs;
use forge_verify::retry::RETRY_VERIFY_ON_CREATE;
use foundry_cli::opts::CoreBuildArgs;
use foundry_common::evm::EvmArgs;
use std::path::PathBuf;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(DebugArgs, opts, evm_opts);

/// CLI arguments for `forge debug`.
#[derive(Clone, Debug, Parser)]
pub struct DebugArgs {
    /// The contract you want to run. Either the file path or contract name.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[arg(value_hint = ValueHint::FilePath)]
    pub path: PathBuf,

    /// Arguments to pass to the script function.
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[arg(long, visible_alias = "tc", value_name = "CONTRACT_NAME")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[arg(long, short, default_value = "run()", value_name = "SIGNATURE")]
    pub sig: String,

    /// Open the script in the debugger.
    #[arg(long)]
    pub debug: bool,

    #[command(flatten)]
    pub opts: CoreBuildArgs,

    #[command(flatten)]
    pub evm_opts: EvmArgs,
}

impl DebugArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let script = ScriptArgs {
            path: self.path.to_str().expect("Invalid path string.").to_string(),
            args: self.args,
            target_contract: self.target_contract,
            sig: self.sig,
            gas_estimate_multiplier: 130,
            opts: self.opts,
            evm_opts: self.evm_opts,
            debug: true,
            retry: RETRY_VERIFY_ON_CREATE,
            ..Default::default()
        };
        script.run_script().await
    }
}
