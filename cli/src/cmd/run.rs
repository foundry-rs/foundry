use crate::{
    cmd::{build::BuildArgs, compile, manual_compile, Cmd},
    opts::forge::EvmOpts,
};
use clap::{Parser, ValueHint};
use ethers::abi::Abi;
use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::{collections::BTreeMap, path::PathBuf};
use ui::{TUIExitReason, Tui, Ui};

use ethers::solc::{
    artifacts::{Optimizer, Settings},
    MinimalCombinedArtifacts, Project, ProjectPathsConfig, SolcConfig,
};

use evm_adapters::Evm;

use ansi_term::Colour;
use ethers::{
    prelude::{artifacts::ContractBytecode, Artifact},
    solc::artifacts::{CompactContractSome, ContractBytecodeSome},
};

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[clap(help = "the path to the contract to run", value_hint = ValueHint::FilePath)]
    pub path: PathBuf,

    #[clap(flatten)]
    pub evm_opts: EvmOpts,

    #[clap(flatten)]
    opts: BuildArgs,

    #[clap(
        long,
        short,
        help = "the contract you want to call and deploy, only necessary if there are more than 1 contract (Interfaces do not count) definitions on the script"
    )]
    pub target_contract: Option<String>,

    #[clap(
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

        let func = IntoFunction::into(self.sig.as_deref().unwrap_or("run()"));
        let BuildOutput { project, contract, highlevel_known_contracts, sources } = self.build()?;

        let known_contracts = highlevel_known_contracts
            .iter()
            .map(|(name, c)| {
                (
                    name.clone(),
                    (
                        c.abi.clone(),
                        c.deployed_bytecode.clone().into_bytes().expect("not bytecode").to_vec(),
                    ),
                )
            })
            .collect::<BTreeMap<String, (Abi, Vec<u8>)>>();

        let CompactContractSome { abi, bin, .. } = contract;
        // this should never fail if compilation was successful
        let bytecode = bin.into_bytes().unwrap();

        // 2. instantiate the EVM w forked backend if needed / pre-funded account(s)
        let mut cfg = crate::utils::sputnik_cfg(self.opts.compiler.evm_version);
        let vicinity = self.evm_opts.vicinity()?;
        let mut evm = crate::utils::sputnik_helpers::evm(&evm_opts, &mut cfg, &vicinity)?;

        // 3. deploy the contract
        let (addr, _, _, logs) = evm.deploy(self.evm_opts.sender, bytecode, 0u32.into())?;

        // 4. set up the runner
        let mut runner =
            ContractRunner::new(&mut evm, &abi, addr, Some(self.evm_opts.sender), &logs);

        // 5. run the test function & potentially the setup
        let needs_setup = abi.functions().any(|func| func.name == "setUp");
        let result = runner.run_test(&func, needs_setup, Some(&known_contracts))?;

        if self.evm_opts.debug {
            // 6. Boot up debugger
            let source_code: BTreeMap<u32, String> = sources
                .iter()
                .map(|(id, path)| {
                    if let Some(resolved) =
                        project.paths.resolve_library_import(&PathBuf::from(path))
                    {
                        (
                            *id,
                            std::fs::read_to_string(resolved).expect(&*format!(
                                "Something went wrong reading the source file: {:?}",
                                path
                            )),
                        )
                    } else {
                        (
                            *id,
                            std::fs::read_to_string(path).expect(&*format!(
                                "Something went wrong reading the source file: {:?}",
                                path
                            )),
                        )
                    }
                })
                .collect();

            let calls = evm.debug_calls();
            println!("debugging");
            let index = if needs_setup && calls.len() > 1 { 1 } else { 0 };
            let mut flattened = Vec::new();
            calls[index].flatten(0, &mut flattened);
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
    pub project: Project<MinimalCombinedArtifacts>,
    pub contract: CompactContractSome,
    pub highlevel_known_contracts: BTreeMap<String, ContractBytecodeSome>,
    pub sources: BTreeMap<u32, String>,
}

impl RunArgs {
    fn target_project(&self) -> eyre::Result<Project<MinimalCombinedArtifacts>> {
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
        Ok(builder.build()?)
    }

    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&self) -> eyre::Result<BuildOutput> {
        let root = dunce::canonicalize(&self.path)?;
        let (project, output) = if let Ok(mut project) = self.opts.project() {
            // TODO: caching causes no output until https://github.com/gakonst/ethers-rs/issues/727
            // is fixed
            project.cached = false;
            project.no_artifacts = true;
            // target contract may not be in the compilation path, add it and manually compile
            match manual_compile(&project, vec![root.clone()]) {
                Ok(output) => (project, output),
                Err(e) => {
                    println!("No extra contracts compiled {:?}", e);
                    let mut target_project = self.target_project()?;
                    target_project.cached = false;
                    target_project.no_artifacts = true;
                    let res = compile(&target_project)?;
                    (target_project, res)
                }
            }
        } else {
            let mut target_project = self.target_project()?;
            target_project.cached = false;
            target_project.no_artifacts = true;
            let res = compile(&target_project)?;
            (target_project, res)
        };
        println!("success.");

        // get the contracts
        let (sources, contracts) = output.output().split();

        // get the specific contract
        let contract_bytecode = if let Some(contract_name) = self.target_contract.clone() {
            let contract_bytecode: ContractBytecode = contracts
                .0
                .get(root.to_str().expect("OsString from path"))
                .ok_or_else(|| {
                    eyre::Error::msg(
                        "contract path not found; This is likely a bug, please report it",
                    )
                })?
                .get(&contract_name)
                .ok_or_else(|| {
                    eyre::Error::msg("contract not found, did you type the name wrong?")
                })?
                .clone()
                .into();
            contract_bytecode.unwrap()
        } else {
            let contract = contracts
                .0
                .get(root.to_str().expect("OsString from path"))
                .ok_or_else(|| {
                    eyre::Error::msg(
                        "contract path not found; This is likely a bug, please report it",
                    )
                })?
                .clone()
                .into_iter()
                .filter_map(|(name, c)| {
                    let c: ContractBytecode = c.into();
                    ContractBytecodeSome::try_from(c).ok().map(|c| (name, c))
                })
                .find(|(_, c)| c.bytecode.object.is_non_empty_bytecode())
                .ok_or_else(|| eyre::Error::msg("no contract found"))?;
            contract.1
        };

        let contract = contract_bytecode.into_compact_contract().unwrap();

        let mut highlevel_known_contracts = BTreeMap::new();

        // build the entire highlevel_known_contracts based on all compiled contracts
        contracts.0.into_iter().for_each(|(src, mapping)| {
            mapping.into_iter().for_each(|(name, c)| {
                let cb: ContractBytecode = c.into();
                if let Ok(cbs) = ContractBytecodeSome::try_from(cb) {
                    if highlevel_known_contracts.contains_key(&name) {
                        highlevel_known_contracts.insert(src.to_string() + ":" + &name, cbs);
                    } else {
                        highlevel_known_contracts.insert(name, cbs);
                    }
                }
            });
        });

        Ok(BuildOutput {
            project,
            contract,
            highlevel_known_contracts,
            sources: sources.into_ids().collect(),
        })
    }
}
