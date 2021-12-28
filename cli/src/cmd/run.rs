use crate::{
    cmd::Cmd,
    opts::forge::{CompilerArgs, EvmOpts},
};
use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::path::PathBuf;
use structopt::StructOpt;

use ethers::{
    prelude::artifacts::CompactContract,
    solc::{
        artifacts::{Optimizer, Settings},
        Project, ProjectPathsConfig, SolcConfig,
    },
};

use evm_adapters::Evm;

use ansi_term::Colour;

#[derive(Debug, Clone, StructOpt)]
pub struct RunArgs {
    #[structopt(help = "the path to the contract to run")]
    pub path: PathBuf,

    #[structopt(flatten)]
    pub compiler: CompilerArgs,

    #[structopt(flatten)]
    pub evm_opts: EvmOpts,

    #[structopt(
        long,
        short,
        help = "the function you want to call on the script contract, defaults to run()"
    )]
    pub sig: Option<String>,

    #[structopt(
        long,
        short,
        help = "the contract you want to call and deploy, only necessary if there are more than 1 contract (Interfaces do not count) definitions on the script"
    )]
    pub contract: Option<String>,

    #[structopt(
        help = "if set to true, skips auto-detecting solc and uses what is in the user's $PATH ",
        long
    )]
    pub no_auto_detect: bool,
}

impl Cmd for RunArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        // Keeping it like this for simplicity.
        #[cfg(not(feature = "sputnik-evm"))]
        unimplemented!("`run` does not work with EVMs other than Sputnik yet");

        let func = IntoFunction::into(self.sig.as_deref().unwrap_or("run()"));
        let (abi, bytecode, _) = self.build()?.into_parts();
        // this should never fail if compilation was successful
        let abi = abi.unwrap();
        let bytecode = bytecode.unwrap();

        // 2. instantiate the EVM w forked backend if needed / pre-funded account(s)
        let mut cfg = crate::utils::sputnik_cfg(self.compiler.evm_version);
        let vicinity = self.evm_opts.vicinity()?;
        let mut evm = crate::utils::sputnik_helpers::evm(&self.evm_opts, &mut cfg, &vicinity)?;

        // 3. deploy the contract
        let (addr, _, _, logs) = evm.deploy(self.evm_opts.sender, bytecode, 0u32.into())?;

        // 4. set up the runner
        let mut runner =
            ContractRunner::new(&mut evm, &abi, addr, Some(self.evm_opts.sender), &logs);

        // 5. run the test function
        let result = runner.run_test(&func, false, None)?;

        // 6. print the result nicely
        if result.success {
            println!("{}", Colour::Green.paint("Script ran successfully."));
        } else {
            println!("{}", Colour::Red.paint("Script failed."));
        }
        println!("Gas Used: {}", result.gas_used);
        println!("== Logs == ");
        result.logs.iter().for_each(|log| println!("{}", log));

        Ok(())
    }
}

impl RunArgs {
    /// Compiles the file with auto-detection and compiler params.
    // TODO: This is too verbose. We definitely want an easier way to do "take this file, detect
    // its solc version and give me all its ABIs & Bytecodes in memory w/o touching disk".
    pub fn build(&self) -> eyre::Result<CompactContract> {
        let paths = ProjectPathsConfig::builder().root(&self.path).sources(&self.path).build()?;

        let optimizer = Optimizer {
            enabled: Some(self.compiler.optimize),
            runs: Some(self.compiler.optimize_runs as usize),
        };

        let solc_settings = Settings {
            optimizer,
            evm_version: Some(self.compiler.evm_version),
            ..Default::default()
        };
        let solc_cfg = SolcConfig::builder().settings(solc_settings).build()?;

        // setup the compiler
        let mut builder = Project::builder()
            .paths(paths)
            .allowed_path(&self.path)
            .solc_config(solc_cfg)
            // we do not want to generate any compilation artifacts in the script run mode
            .no_artifacts()
            // no cache
            .ephemeral();
        if self.no_auto_detect {
            builder = builder.no_auto_detect();
        }
        let project = builder.build()?;

        println!("compiling...");
        let output = project.compile()?;
        if output.has_compiler_errors() {
            // return the diagnostics error back to the user.
            eyre::bail!(output.to_string())
        } else if output.is_unchanged() {
            println!("no files changed, compilation skippped.");
        } else {
            println!("success.");
        };

        // get the contracts
        let contracts = output.output();

        // get the specific contract
        let contract = if let Some(ref contract) = self.contract {
            let contract = contracts.find(contract).ok_or_else(|| {
                eyre::Error::msg("contract not found, did you type the name wrong?")
            })?;
            CompactContract::from(contract)
        } else {
            let mut contracts = contracts.contracts_into_iter().filter(|(_, contract)| {
                // TODO: Should have a helper function for finding if a contract's bytecode is
                // empty or not.
                match contract.evm {
                    Some(ref evm) => match evm.bytecode {
                        Some(ref bytecode) => bytecode
                            .object
                            .as_bytes()
                            .map(|x| !x.as_ref().is_empty())
                            .unwrap_or(false),
                        _ => false,
                    },
                    _ => false,
                }
            });
            let contract = contracts.next().ok_or_else(|| eyre::Error::msg("no contract found"))?.1;
            if contracts.peekable().peek().is_some() {
                eyre::bail!(
                    ">1 contracts found, please provide a contract name to choose one of them"
                )
            }
            CompactContract::from(contract)
        };
        Ok(contract)
    }
}
