use crate::{
    cmd::{build::BuildArgs, compile_files, Cmd},
    opts::evm::EvmArgs,
};
use ansi_term::Colour;
use clap::{Parser, ValueHint};
use ethers::{
    abi::{Abi, RawLog},
    solc::{
        artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
        Project,
    },
    types::{Address, Bytes, U256},
};
use forge::{
    debugger::DebugArena,
    decode::decode_console_logs,
    executor::{
        opts::EvmOpts, CallResult, DatabaseRef, DeployResult, EvmError, Executor, ExecutorBuilder,
        Fork, RawCallResult,
    },
    trace::{identifier::LocalTraceIdentifier, CallTraceArena, CallTraceDecoder, TraceKind},
    CALLER,
};
use foundry_config::{figment::Figment, Config};
use foundry_utils::{encode_args, IntoFunction, PostLinkInput};
use std::{collections::BTreeMap, path::PathBuf};
use ui::{TUIExitReason, Tui, Ui};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(RunArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[clap(help = "the path of the contract to run", value_hint = ValueHint::FilePath)]
    pub path: PathBuf,

    pub args: Vec<String>,

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
        default_value = "run()",
        help = "the function you want to call on the script contract, defaults to run()"
    )]
    pub sig: String,
}

impl Cmd for RunArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let verbosity = evm_opts.verbosity;
        let config = Config::from_provider(figment).sanitized();

        let BuildOutput {
            project,
            contract,
            highlevel_known_contracts,
            sources,
            predeploy_libraries,
        } = self.build(&config, &evm_opts)?;

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

        let CompactContractBytecode { abi, bytecode, .. } = contract;
        let abi = abi.expect("no ABI for contract");
        let bytecode = bytecode.expect("no bytecode for contract").object.into_bytes().unwrap();
        let needs_setup = abi.functions().any(|func| func.name == "setUp");

        let mut builder = ExecutorBuilder::new()
            .with_cheatcodes(evm_opts.ffi)
            .with_config(evm_opts.evm_env())
            .with_spec(crate::utils::evm_spec(&config.evm_version));
        if let Some(ref url) = self.evm_opts.fork_url {
            let fork = Fork { url: url.clone(), pin_block: self.evm_opts.fork_block_number };
            builder = builder.with_fork(fork);
        }
        if verbosity >= 3 {
            builder = builder.with_tracing();
        }
        if evm_opts.debug {
            builder = builder.with_tracing().with_debugger();
        }

        let mut result = {
            let mut runner =
                Runner::new(builder.build(), evm_opts.initial_balance, evm_opts.sender);
            let (address, mut result) =
                runner.setup(&predeploy_libraries, bytecode, needs_setup)?;

            let RunResult {
                success,
                gas_used,
                logs,
                traces,
                debug: run_debug,
                labeled_addresses,
                ..
            } = runner
                .run(address, encode_args(&IntoFunction::into(self.sig), &self.args)?.into())?;

            result.success &= success;

            result.gas_used = gas_used;
            result.logs.extend(logs);
            result.traces.extend(traces);
            result.debug = run_debug;
            result.labeled_addresses.extend(labeled_addresses);

            result
        };

        // Identify addresses in each trace
        let local_identifier = LocalTraceIdentifier::new(&known_contracts);
        let mut decoder = CallTraceDecoder::new_with_labels(result.labeled_addresses.clone());
        for (_, trace) in &mut result.traces {
            decoder.identify(trace, &local_identifier);
        }

        if evm_opts.debug {
            let source_code: BTreeMap<u32, String> = sources
                .iter()
                .map(|(id, path)| {
                    let resolved = project
                        .paths
                        .resolve_library_import(&PathBuf::from(path))
                        .unwrap_or_else(|| PathBuf::from(path));
                    (
                        *id,
                        std::fs::read_to_string(resolved).expect(&*format!(
                            "Something went wrong reading the source file: {:?}",
                            path
                        )),
                    )
                })
                .collect();

            let calls: Vec<DebugArena> = result.debug.expect("we should have collected debug info");
            let flattened = calls.last().expect("we should have collected debug info").flatten(0);
            let tui =
                Tui::new(flattened, 0, decoder.contracts, highlevel_known_contracts, source_code)?;
            match tui.start().expect("Failed to start tui") {
                TUIExitReason::CharExit => return Ok(()),
            }
        } else {
            if verbosity >= 3 {
                if result.traces.is_empty() {
                    eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/gakonst/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
                }

                if !result.success && verbosity == 3 || verbosity > 3 {
                    println!("Traces:");
                    for (kind, trace) in &mut result.traces {
                        let should_include = match kind {
                            TraceKind::Setup => {
                                (verbosity >= 5) || (verbosity == 4 && !result.success)
                            }
                            TraceKind::Execution => verbosity > 3 || !result.success,
                            _ => false,
                        };

                        if should_include {
                            decoder.decode(trace);
                            println!("{}", trace);
                        }
                    }
                    println!();
                }
            }

            if result.success {
                println!("{}", Colour::Green.paint("Script ran successfully."));
            } else {
                println!("{}", Colour::Red.paint("Script failed."));
            }

            println!("Gas used: {}", result.gas_used);
            println!("== Logs ==");
            let console_logs = decode_console_logs(&result.logs);
            if !console_logs.is_empty() {
                for log in console_logs {
                    println!("  {}", log);
                }
            }
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
    pub project: Project,
    pub contract: CompactContractBytecode,
    pub highlevel_known_contracts: BTreeMap<String, ContractBytecodeSome>,
    pub sources: BTreeMap<u32, String>,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
}

impl RunArgs {
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&self, config: &Config, evm_opts: &EvmOpts) -> eyre::Result<BuildOutput> {
        let target_contract = dunce::canonicalize(&self.path)?;
        let project = config.ephemeral_no_artifacts_project()?;
        let output = compile_files(&project, vec![target_contract])?;

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

struct RunResult {
    pub success: bool,
    pub logs: Vec<RawLog>,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: Option<Vec<DebugArena>>,
    pub gas_used: u64,
    pub labeled_addresses: BTreeMap<Address, String>,
}

struct Runner<DB: DatabaseRef> {
    pub executor: Executor<DB>,
    pub initial_balance: U256,
    pub sender: Address,
}

impl<DB: DatabaseRef> Runner<DB> {
    pub fn new(executor: Executor<DB>, initial_balance: U256, sender: Address) -> Self {
        Self { executor, initial_balance, sender }
    }

    pub fn setup(
        &mut self,
        libraries: &[Bytes],
        code: Bytes,
        setup: bool,
    ) -> eyre::Result<(Address, RunResult)> {
        // We max out their balance so that they can deploy and make calls.
        self.executor.set_balance(self.sender, U256::MAX);
        self.executor.set_balance(*CALLER, U256::MAX);

        // We set the nonce of the deployer accounts to 1 to get the same addresses as DappTools
        self.executor.set_nonce(self.sender, 1);

        // Deploy libraries
        let mut traces: Vec<(TraceKind, CallTraceArena)> = libraries
            .iter()
            .filter_map(|code| {
                let DeployResult { traces, .. } = self
                    .executor
                    .deploy(self.sender, code.0.clone(), 0u32.into())
                    .expect("couldn't deploy library");

                traces
            })
            .map(|traces| (TraceKind::Deployment, traces))
            .collect();

        // Deploy an instance of the contract
        let DeployResult {
            address,
            mut logs,
            traces: constructor_traces,
            debug: constructor_debug,
            ..
        } = self.executor.deploy(self.sender, code.0, 0u32.into()).expect("couldn't deploy");
        traces.extend(constructor_traces.map(|traces| (TraceKind::Deployment, traces)).into_iter());
        self.executor.set_balance(address, self.initial_balance);

        // Optionally call the `setUp` function
        Ok(if setup {
            match self.executor.setup(address) {
                Ok(CallResult {
                    reverted,
                    traces: setup_traces,
                    labels,
                    logs: setup_logs,
                    debug,
                    gas: gas_used,
                    ..
                }) |
                Err(EvmError::Execution {
                    reverted,
                    traces: setup_traces,
                    labels,
                    logs: setup_logs,
                    debug,
                    gas_used,
                    ..
                }) => {
                    traces
                        .extend(setup_traces.map(|traces| (TraceKind::Setup, traces)).into_iter());
                    logs.extend_from_slice(&setup_logs);

                    (
                        address,
                        RunResult {
                            logs,
                            traces,
                            labeled_addresses: labels,
                            success: !reverted,
                            debug: vec![constructor_debug, debug].into_iter().collect(),
                            gas_used,
                        },
                    )
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            (
                address,
                RunResult {
                    logs,
                    traces,
                    success: true,
                    debug: vec![constructor_debug].into_iter().collect(),
                    gas_used: 0,
                    labeled_addresses: Default::default(),
                },
            )
        })
    }

    pub fn run(&mut self, address: Address, calldata: Bytes) -> eyre::Result<RunResult> {
        let RawCallResult { reverted, gas, logs, traces, labels, debug, .. } =
            self.executor.call_raw(self.sender, address, calldata.0, 0.into())?;
        Ok(RunResult {
            success: !reverted,
            gas_used: gas,
            logs,
            traces: traces.map(|traces| vec![(TraceKind::Execution, traces)]).unwrap_or_default(),
            debug: vec![debug].into_iter().collect(),
            labeled_addresses: labels,
        })
    }
}
