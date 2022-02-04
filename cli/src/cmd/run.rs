use crate::cmd::{build::BuildArgs, compile, manual_compile, Cmd};
use clap::{Parser, ValueHint};
use evm_adapters::sputnik::cheatcodes::{CONSOLE_ABI, HEVMCONSOLE_ABI, HEVM_ABI};

use forge::ContractRunner;
use foundry_utils::IntoFunction;
use std::{collections::BTreeMap, path::PathBuf};
use ui::{TUIExitReason, Tui, Ui};

use ethers::solc::{MinimalCombinedArtifacts, Project};

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
        let BuildOutput {
            project,
            contract,
            highlevel_known_contracts,
            sources,
            predeploy_libraries,
        } = self.build(config, &evm_opts)?;

        let mut known_contracts = highlevel_known_contracts
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

        known_contracts.insert("VM".to_string(), (HEVM_ABI.clone(), Vec::new()));
        known_contracts.insert("VM_CONSOLE".to_string(), (HEVMCONSOLE_ABI.clone(), Vec::new()));
        known_contracts.insert("CONSOLE".to_string(), (CONSOLE_ABI.clone(), Vec::new()));

        let CompactContractBytecode { abi, bytecode, .. } = contract;
        let abi = abi.expect("No abi for contract");
        let bytecode = bytecode.expect("No bytecode").object.into_bytes().unwrap();
        let needs_setup = abi.functions().any(|func| func.name == "setUp");

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
                    &abi,
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
                    let mut exec_info = ExecutionInfo::new(
                        &known_contracts,
                        &mut ident,
                        &result.labeled_addresses,
                        &funcs,
                        &events,
                        &errors,
                    );
                    let vm = vm();
                    let mut trace_string = "".to_string();
                    if evm_opts.verbosity > 4 || !result.success {
                        // print setup calls as well
                        traces.iter().for_each(|trace| {
                            trace.construct_trace_string(
                                0,
                                &mut exec_info,
                                &vm,
                                "",
                                &mut trace_string,
                            );
                        });
                    } else if !traces.is_empty() {
                        traces.last().expect("no last but not empty").construct_trace_string(
                            0,
                            &mut exec_info,
                            &vm,
                            "",
                            &mut trace_string,
                        );
                    }
                    if !trace_string.is_empty() {
                        println!("{}", trace_string);
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
                println!();
            } else if result.traces.is_none() {
                eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/gakonst/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
            } else if result.identified_contracts.is_none() {
                eyre::bail!(
                    "Unexpected error: No identified contracts. Please report this as a bug: https://github.com/gakonst/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml"
                );
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

struct ExtraLinkingInfo<'a> {
    no_target_name: bool,
    target_fname: String,
    contract: &'a mut CompactContractBytecode,
    dependencies: &'a mut Vec<ethers::types::Bytes>,
    matched: bool,
}

pub struct BuildOutput {
    pub project: Project<MinimalCombinedArtifacts>,
    pub contract: CompactContractBytecode,
    pub highlevel_known_contracts: BTreeMap<String, ContractBytecodeSome>,
    pub sources: BTreeMap<u32, String>,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
}

impl RunArgs {
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&self, config: Config, evm_opts: &EvmOpts) -> eyre::Result<BuildOutput> {
        let target_contract = dunce::canonicalize(&self.path)?;
        let (project, output) = if let Ok(mut project) = config.project() {
            // TODO: caching causes no output until https://github.com/gakonst/ethers-rs/issues/727
            // is fixed
            project.cached = false;
            project.no_artifacts = true;

            // target contract may not be in the compilation path, add it and manually compile
            match manual_compile(&project, vec![target_contract]) {
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

        let (sources, all_contracts) = output.output().split();

        let contracts: BTreeMap<String, CompactContractBytecode> = all_contracts
            .contracts_with_files()
            .map(|(file, name, contract)| (format!("{}:{}", file, name), contract.clone().into()))
            .collect();

        let mut run_dependencies = vec![];
        let mut contract =
            CompactContractBytecode { abi: None, bytecode: None, deployed_bytecode: None };
        let mut highlevel_known_contracts = BTreeMap::new();

        let mut target_fname = dunce::canonicalize(&self.path)
            .expect("Couldn't convert contract path to absolute path")
            .to_str()
            .expect("Bad path to string")
            .to_string();

        let no_target_name = if let Some(target_name) = &self.target_contract {
            target_fname = target_fname + ":" + target_name;
            false
        } else {
            true
        };

        foundry_utils::link(
            &contracts,
            &mut highlevel_known_contracts,
            evm_opts.sender,
            &mut ExtraLinkingInfo {
                no_target_name,
                target_fname,
                contract: &mut contract,
                dependencies: &mut run_dependencies,
                matched: false,
            },
            |file, key| (format!("{}:{}", file, key), file, key),
            |post_link_input| {
                let PostLinkInput {
                    contract,
                    known_contracts: highlevel_known_contracts,
                    fname,
                    extra,
                    dependencies,
                } = post_link_input;
                let split = fname.split(':').collect::<Vec<&str>>();

                // if its the target contract, grab the info
                if extra.no_target_name && split[0] == extra.target_fname {
                    if extra.matched {
                        eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `-t ContractName`")
                    }
                    *extra.dependencies = dependencies;
                    *extra.contract = contract.clone();
                    extra.matched = true;
                } else if extra.target_fname == fname {
                    *extra.dependencies = dependencies;
                    *extra.contract = contract.clone();
                    extra.matched = true;
                }

                let tc: ContractBytecode = contract.into();
                let contract_name = if split.len() > 1 { split[1] } else { split[0] };
                highlevel_known_contracts.insert(contract_name.to_string(), tc.unwrap());
                Ok(())
            },
        )?;

        Ok(BuildOutput {
            project,
            contract,
            highlevel_known_contracts,
            sources: sources.into_ids().collect(),
            predeploy_libraries: run_dependencies,
        })
    }
}
