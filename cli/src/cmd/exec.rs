use crate::cmd::{build::BuildArgs, compile_files, Cmd};
use clap::{Parser, ValueHint};
use evm_adapters::sputnik::cheatcodes::{CONSOLE_ABI, HEVMCONSOLE_ABI, HEVM_ABI};

use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::{collections::BTreeMap, path::PathBuf};
use ui::{TUIExitReason, Tui, Ui};

use ethers::solc::Project;

use crate::opts::evm::EvmArgs;
use ansi_term::Colour;
use ethers::{
    abi::Abi,
    solc::artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
};
use evm_adapters::{
    call_tracing::ExecutionInfo,
    evm_opts::{BackendKind, EvmOpts},
    sputnik::{cheatcodes::debugger::DebugArena, helpers::vm},
};
use foundry_config::{figment::Figment, Config};
use foundry_utils::PostLinkInput;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(ExecArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct ExecArgs {
    #[clap(help = "the bytecode to execute")]
    pub bytecode: String,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,

    #[clap(flatten)]
    opts: BuildArgs,
}

impl Cmd for ExecArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        // Keeping it like this for simplicity.
        #[cfg(not(feature = "sputnik-evm"))]
        unimplemented!("`exec` does not work with EVMs other than Sputnik yet");

        let figment: Figment = From::from(&self);
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();
        let evm_version = config.evm_version;
        if evm_opts.debug {
            evm_opts.verbosity = 3;
        }

        // TODO: parse bytecode string

        let mut cfg = crate::utils::sputnik_cfg(&evm_version);
        cfg.create_contract_limit = None;
        let vicinity = evm_opts.vicinity()?;
        let backend = evm_opts.backend(&vicinity)?;

        // need to match on the backend type
        let result = match backend {
            BackendKind::Simple(ref backend) => {
                let runner = ContractRunner::new(
                    &evm_opts,
                    &cfg,
                    backend,
                    &Contract::default(),
                    bytecode,
                    Some(evm_opts.sender),
                    None,
                    &predeploy_libraries,
                );
                runner.run_test(&func, needs_setup, Some(&known_contracts))?
            }
            BackendKind::Shared(ref backend) => {
                let runner = ContractRunner::new(
                    &evm_opts,
                    &cfg,
                    backend,
                    &abi,
                    bytecode,
                    Some(evm_opts.sender),
                    None,
                    &predeploy_libraries,
                );
                runner.run_test(&func, needs_setup, Some(&known_contracts))?
            }
        };

        // TODO: support evm_opts.debug and tracing (?)

        if result.success {
            println!("{}", Colour::Green.paint("Script ran successfully."));
        } else {
            println!("{}", Colour::Red.paint("Script failed."));
        }

        println!("Gas Used: {}", result.gas_used);
        println!("== Logs ==");
        result.logs.iter().for_each(|log| println!("{}", log));

        Ok(())
    }
}

struct ExtraLinkingInfo<'a> {
    no_target_name: bool,
    target_fname: String,
    contract: &'a mut CompactContractBytecode,
    dependencies: &'a mut Vec<ethers::types::Bytes>,
    matched: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BuildOutput {
    pub project: Project,
    pub contract: CompactContractBytecode,
    pub highlevel_known_contracts: BTreeMap<String, ContractBytecodeSome>,
    pub sources: BTreeMap<u32, String>,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
}

impl ExecArgs {
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&self, config: Config, evm_opts: &EvmOpts) -> eyre::Result<BuildOutput> {
        // TODO: ??

        Ok(BuildOutput::default())
    }
}
