use crate::{
    cmd::{forge::build::CoreBuildArgs, Cmd},
    opts::MultiWallet,
};
use clap::{Parser, ValueHint};

use foundry_common::evm::EvmArgs;

use foundry_utils::RuntimeOrHandle;
use std::path::PathBuf;

use super::{build::BuildArgs, script::ScriptArgs};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(DebugArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct DebugArgs {
    /// The path of the contract to run.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[clap(value_hint = ValueHint::FilePath)]
    pub path: PathBuf,

    /// Arguments to pass to the script function.
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[clap(long, short, value_name = "CONTRACT_NAME")]
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

impl Cmd for DebugArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let script = ScriptArgs {
            path: self.path,
            args: self.args,
            target_contract: self.target_contract,
            sig: self.sig,
            legacy: false,
            broadcast: false,
            opts: BuildArgs {
                args: self.opts,
                names: false,
                sizes: false,
                watch: Default::default(),
            },
            wallets: MultiWallet {
                interactives: 0,
                private_keys: None,
                mnemonic_paths: None,
                mnemonic_indexes: None,
                keystore_paths: None,
                keystore_passwords: None,
                ledger: false,
                trezor: false,
                hd_path: None,
                froms: None,
            },
            evm_opts: self.evm_opts,
            resume: false,
            force_resume: false,
            deployer: None,
            debug: true,
        };
        RuntimeOrHandle::new().block_on(script.run_script())
    }
}
