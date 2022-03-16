use crate::cmd::{forge::build::BuildArgs, Cmd};
use clap::Parser;

use forge::ContractRunner;
use foundry_utils::IntoFunction;

use ethers::prelude::Bytes;

use crate::opts::evm::EvmArgs;
use ansi_term::Colour;
use evm_adapters::evm_opts::{BackendKind, EvmOpts};
use foundry_config::{figment::Figment, Config};

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

        let mut cfg = crate::utils::sputnik_cfg(&evm_version);
        cfg.create_contract_limit = None;
        let vicinity = evm_opts.vicinity()?;
        let backend = evm_opts.backend(&vicinity)?;

        // Parse bytecode string
        let bytecode_vec = self.bytecode.strip_prefix("0x").unwrap_or(&self.bytecode);
        let parsed_bytecode = Bytes::from(hex::decode(bytecode_vec)?);

        // need to match on the backend type
        let func = IntoFunction::into("test()");
        let contract = Default::default();
        let predeploy_libs = Vec::new();
        let result = match backend {
            BackendKind::Simple(ref backend) => {
                let runner = ContractRunner::new(
                    &evm_opts,
                    &cfg,
                    backend,
                    &contract,
                    parsed_bytecode,
                    Some(evm_opts.sender),
                    None,
                    &predeploy_libs,
                );
                runner.run_test(&func, false, None)?
            }
            BackendKind::Shared(ref backend) => {
                let runner = ContractRunner::new(
                    &evm_opts,
                    &cfg,
                    backend,
                    &contract,
                    parsed_bytecode,
                    Some(evm_opts.sender),
                    None,
                    &predeploy_libs,
                );
                runner.run_test(&func, false, None)?
            }
        };

        // TODO: support evm_opts.debug and tracing
        println!("Full result: {:?}", result);

        if result.success {
            println!("{}", Colour::Green.paint("Bytecode executed successfully."));
        } else {
            println!("{}", Colour::Red.paint("Bytecode failed."));
        }

        println!("Gas Used: {}", result.gas_used);
        println!("== Logs ==");
        result.logs.iter().for_each(|log| println!("{}", log));

        Ok(())
    }
}

impl ExecArgs {
    pub fn build(&self, _: Config, _: &EvmOpts) -> eyre::Result<()> {
        Ok(())
    }
}
