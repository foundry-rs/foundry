use crate::cmd::{build::BuildArgs, compile, manual_compile, Cmd};
use clap::{Parser, ValueHint};
use ethers::abi::Abi;
use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::{collections::BTreeMap, path::PathBuf};
use ui::{TUIExitReason, Tui, Ui};

use ethers::solc::{MinimalCombinedArtifacts, Project};

use crate::opts::evm::EvmArgs;
use ansi_term::Colour;
use ethers::{
    prelude::{artifacts::ContractBytecode, Artifact},
    solc::artifacts::{CompactContractSome, ContractBytecodeSome},
};
use evm_adapters::{
    call_tracing::ExecutionInfo,
    evm_opts::{BackendKind, EvmOpts},
    sputnik::{cheatcodes::debugger::DebugArena, helpers::vm},
};
use foundry_config::{figment::Figment, Config};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(RunArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[clap(help = "the path to the contract to run", value_hint = ValueHint::FilePath)]
    pub path: PathBuf,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,

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

        let figment: Figment = From::from(&self);
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();
        let evm_version = config.evm_version;
        if evm_opts.debug {
            evm_opts.verbosity = 3;
        }

        let func = IntoFunction::into(self.sig.as_deref().unwrap_or("run()"));
        let BuildOutput { project, contract, highlevel_known_contracts, sources } =
            self.build(config)?;

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
        let needs_setup = abi.functions().any(|func| func.name == "setUp");

        let cfg = crate::utils::sputnik_cfg(&evm_version);
        let vicinity = evm_opts.vicinity()?;
        let backend = evm_opts.backend(&vicinity)?;

        // need to match on the backend type
        let result = match backend {
            BackendKind::Simple(ref backend) => {
                let runner = ContractRunner::new(
                    &evm_opts,
                    &cfg,
                    backend,
                    &abi,
                    bytecode,
                    Some(evm_opts.sender),
                    None,
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
                );
                runner.run_test(&func, needs_setup, Some(&known_contracts))?
            }
        };

        if evm_opts.debug {
            // 4. Boot up debugger
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

            let calls: Vec<DebugArena> = result.debug_calls.expect("Debug must be enabled by now");
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
                    let (funcs, events, errors) =
                        foundry_utils::flatten_known_contracts(&known_contracts);
                    let mut exec_info =
                        ExecutionInfo::new(&known_contracts, &mut ident, &funcs, &events, &errors);
                    let vm = vm();
                    if evm_opts.verbosity > 4 || !result.success {
                        // print setup calls as well
                        traces.iter().for_each(|trace| {
                            trace.pretty_print(0, &mut exec_info, &vm, "");
                        });
                    } else if !traces.is_empty() {
                        traces.last().expect("no last but not empty").pretty_print(
                            0,
                            &mut exec_info,
                            &vm,
                            "",
                        );
                    }
                }
                println!();
            }
        } else {
            // 5. print the result nicely
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
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&self, config: Config) -> eyre::Result<BuildOutput> {
        let target_contract = dunce::canonicalize(&self.path)?;
        let (project, output) = if let Ok(mut project) = config.project() {
            // TODO: caching causes no output until https://github.com/gakonst/ethers-rs/issues/727
            // is fixed
            project.cached = false;
            project.no_artifacts = true;

            // target contract may not be in the compilation path, add it and manually compile
            match manual_compile(&project, vec![target_contract.clone()]) {
                Ok(output) => (project, output),
                Err(e) => {
                    println!("No extra contracts compiled {:?}", e);
                    let mut target_project = config.ephemeral_no_artifacts_project()?;
                    target_project.cached = false;
                    target_project.no_artifacts = true;
                    let res = compile(&target_project)?;
                    (target_project, res)
                }
            }
        } else {
            let mut target_project = config.ephemeral_no_artifacts_project()?;
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
                .get(target_contract.to_str().expect("OsString from path"))
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
                .get(target_contract.to_str().expect("OsString from path"))
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
