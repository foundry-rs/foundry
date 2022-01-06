use crate::{
    cmd::{build::BuildArgs, compile, Cmd},
    opts::forge::EvmOpts,
};
use ethers::{
    abi::Abi,
    prelude::artifacts::{Bytecode, DeployedBytecode},
};
use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::{collections::BTreeMap, path::PathBuf};
use structopt::StructOpt;
use ui::{TUIExitReason, Tui, Ui};

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
    pub evm_opts: EvmOpts,

    #[structopt(flatten)]
    opts: BuildArgs,

    #[structopt(
        long,
        short,
        help = "the contract you want to call and deploy, only necessary if there are more than 1 contract (Interfaces do not count) definitions on the script"
    )]
    pub target_contract: Option<String>,

    #[structopt(
        long,
        short,
        help = "the function you want to call on the script contract, defaults to run()"
    )]
    pub sig: Option<String>,
}

impl Cmd for RunArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        // Keeping it like this for simplicity.
        #[cfg(not(feature = "sputnik-evm"))]
        unimplemented!("`run` does not work with EVMs other than Sputnik yet");

        let mut evm_opts = self.evm_opts.clone();
        if evm_opts.debug {
            evm_opts.verbosity = 3;
        }

        // This is just the contracts compiled, but we need to merge this with the read cached
        // artifacts

        let func = IntoFunction::into(self.sig.as_deref().unwrap_or("run()"));
        let BuildOutput { contract, mut highlevel_known_contracts, sources } = self.build()?;

        // if we have a high verbosity, we want all possible compiler data not just for this
        // contract in case the transaction interacts with others
        if evm_opts.debug || evm_opts.verbosity > 3 {
            if let Ok(project) = self.opts.project() {
                println!("Compiling full repo to aid in debugging/tracing");
                if let Ok(output) = compile(&project) {
                    let contracts = output.output();
                    for (contract_name, contract) in contracts.contracts_into_iter() {
                        insert_contract_info(
                            &mut highlevel_known_contracts,
                            contract_name.to_string(),
                            &contract,
                        );
                    }
                } else {
                    println!("No extra contracts compiled");
                }
            }
        }

        let known_contracts = highlevel_known_contracts
            .iter()
            .map(|(name, (abi, _b, deployed_b))| {
                (
                    name.clone(),
                    (
                        abi.clone(),
                        deployed_b
                            .clone()
                            .bytecode
                            .expect("no bytes")
                            .object
                            .into_bytes()
                            .expect("not bytecode")
                            .to_vec(),
                    ),
                )
            })
            .collect::<BTreeMap<String, (Abi, Vec<u8>)>>();
        let (abi, bytecode, runtime_bytecode) = contract.into_parts();

        // this should never fail if compilation was successful
        let abi = abi.unwrap();
        let bytecode = bytecode.unwrap();
        let _runtime_bytecode = runtime_bytecode.unwrap();

        // 2. instantiate the EVM w forked backend if needed / pre-funded account(s)
        let mut cfg = crate::utils::sputnik_cfg(self.opts.compiler.evm_version);
        let vicinity = self.evm_opts.vicinity()?;
        let mut evm = crate::utils::sputnik_helpers::evm(&evm_opts, &mut cfg, &vicinity)?;

        // 3. deploy the contract
        let (addr, _, _, logs) = evm.deploy(self.evm_opts.sender, bytecode, 0u32.into())?;

        // 4. set up the runner
        let mut runner =
            ContractRunner::new(&mut evm, &abi, addr, Some(self.evm_opts.sender), &logs);

        // 5. run the test function
        let result = runner.run_test(&func, false, Some(&known_contracts))?;

        if self.evm_opts.debug {
            // 6. Boot up debugger
            let source_code: BTreeMap<u32, String> = sources
                .iter()
                .map(|(id, path)| {
                    (
                        *id,
                        std::fs::read_to_string(path)
                            .expect("Something went wrong reading the file"),
                    )
                })
                .collect();

            let calls = evm.debug_calls();
            println!("debugging {}", calls.len());
            let mut flattened = Vec::new();
            calls[0].flatten(0, &mut flattened);
            flattened = flattened[1..].to_vec();
            let tui = Tui::new(
                flattened,
                0,
                result.identified_contracts.expect("debug but not verbosity"),
                highlevel_known_contracts,
                source_code,
            )?;
            match tui.start().expect("Failed to start tui") {
                TUIExitReason::CharExit => return Ok(()),
            }
        } else if evm_opts.verbosity > 2 {
            // support traces
            if let (Some(traces), Some(identified_contracts)) =
                (&result.traces, &result.identified_contracts)
            {
                if !result.success && evm_opts.verbosity == 3 || evm_opts.verbosity > 3 {
                    let mut ident = identified_contracts.clone();
                    if evm_opts.verbosity > 4 || !result.success {
                        // print setup calls as well
                        traces.iter().for_each(|trace| {
                            trace.pretty_print(0, &known_contracts, &mut ident, runner.evm, "");
                        });
                    } else if !traces.is_empty() {
                        traces.last().expect("no last but not empty").pretty_print(
                            0,
                            &known_contracts,
                            &mut ident,
                            runner.evm,
                            "",
                        );
                    }
                }

                println!();
            }
        } else {
            // 6. print the result nicely
            if result.success {
                println!("{}", Colour::Green.paint("Script ran successfully."));
            } else {
                println!("{}", Colour::Red.paint("Script failed."));
            }

            println!("Gas Used: {}", result.gas_used);
            println!("== Logs == ");
            result.logs.iter().for_each(|log| println!("{}", log));
        }

        Ok(())
    }
}

pub struct BuildOutput {
    pub contract: CompactContract,
    pub highlevel_known_contracts: BTreeMap<String, (Abi, Bytecode, DeployedBytecode)>,
    pub sources: BTreeMap<u32, String>,
}

impl RunArgs {
    /// Compiles the file with auto-detection and compiler params.
    // TODO: This is too verbose. We definitely want an easier way to do "take this file, detect
    // its solc version and give me all its ABIs & Bytecodes in memory w/o touching disk".
    pub fn build(&self) -> eyre::Result<BuildOutput> {
        let paths = ProjectPathsConfig::builder().root(&self.path).sources(&self.path).build()?;

        let optimizer = Optimizer {
            enabled: Some(self.opts.compiler.optimize),
            runs: Some(self.opts.compiler.optimize_runs as usize),
        };

        let solc_settings = Settings {
            optimizer,
            evm_version: Some(self.opts.compiler.evm_version),
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
        if self.opts.no_auto_detect {
            builder = builder.no_auto_detect();
        }
        let project = builder.build()?;
        let output = compile(&project)?;

        // get the contracts
        let contracts = output.output();
        let sources = contracts
            .sources
            .iter()
            .map(|(path, source_file)| (source_file.id, path.clone()))
            .collect();

        // deployed bytecode one for
        let mut highlevel_known_contracts: BTreeMap<String, (Abi, Bytecode, DeployedBytecode)> =
            Default::default();

        // get the specific contract
        let contract = if let Some(ref contract_name) = self.target_contract {
            let (_name, contract) = contracts
                .contracts_into_iter()
                .find(|(name, _contract)| name == contract_name)
                .ok_or_else(|| {
                    eyre::Error::msg("contract not found, did you type the name wrong?")
                })?;
            insert_contract_info(
                &mut highlevel_known_contracts,
                contract_name.to_string(),
                &contract,
            );
            CompactContract::from(contract)
        } else {
            let mut contracts = contracts.contracts_into_iter().filter(|(_fname, contract)| {
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
            let (contract_name, contract) =
                contracts.next().ok_or_else(|| eyre::Error::msg("no contract found"))?;
            insert_contract_info(&mut highlevel_known_contracts, contract_name, &contract);
            if contracts.peekable().peek().is_some() {
                eyre::bail!(
                    ">1 contracts found, please provide a contract name to choose one of them"
                )
            }
            CompactContract::from(contract)
        };
        Ok(BuildOutput { contract, highlevel_known_contracts, sources })
    }
}

fn insert_contract_info(
    highlevel_known_contracts: &mut BTreeMap<String, (Abi, Bytecode, DeployedBytecode)>,
    name: String,
    contract: &ethers::solc::artifacts::Contract,
) {
    let abi = contract.abi.clone().expect("no abi");
    let evm = contract.evm.clone().expect("no evm");
    let creation_bytecode = evm.bytecode.expect("no creation bytecode");
    let runtime_bytecode = evm.deployed_bytecode.expect("no deployed bytecode");
    highlevel_known_contracts.insert(name, (abi, creation_bytecode, runtime_bytecode));
}
