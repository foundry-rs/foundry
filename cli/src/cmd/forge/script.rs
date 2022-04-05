use crate::{
    cmd::{compile_files, forge::build::BuildArgs, Cmd},
    opts::{Wallet, evm::EvmArgs},
    utils,
};
use ansi_term::Colour;
use clap::{Parser, ValueHint};
use ethers::{abi::{Abi, RawLog}, prelude::{ArtifactId}, solc::{
        artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
        Project,
    }, types::{Address, Bytes, U256, transaction::eip2718::TypedTransaction}};
use forge::{
    debug::DebugArena,
    decode::decode_console_logs,
    executor::{
        builder::Backend, opts::EvmOpts, CallResult, DatabaseRef, DeployResult, EvmError, Executor,
        ExecutorBuilder, RawCallResult,
    },
    trace::{identifier::LocalTraceIdentifier, CallTraceArena, CallTraceDecoder, TraceKind},
    CALLER,
};
use foundry_config::{figment::Figment, Config};
use foundry_utils::{encode_args, IntoFunction, PostLinkInput};
use std::{collections::{BTreeMap, VecDeque}, path::PathBuf};


// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(ScriptArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct ScriptArgs {
    /// The path of the contract to run.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[clap(value_hint = ValueHint::FilePath)]
    pub path: PathBuf,

    /// Arguments to pass to the script function.
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[clap(long, alias = "tc")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[clap(long, short, default_value = "run()")]
    pub sig: String,

    #[clap(
        long,
        help = "use legacy transactions instead of EIP1559 ones. this is auto-enabled for common networks without EIP1559"
    )]
    pub legacy: bool,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    pub opts: BuildArgs,

    #[clap(flatten)]
    pub wallet: Wallet,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,
}

impl Cmd for ScriptArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let verbosity = evm_opts.verbosity;
        let config = Config::from_provider(figment).sanitized();

        let BuildOutput {
            project: _,
            contract,
            highlevel_known_contracts,
            sources: _,
            predeploy_libraries,
        } = self.build(&config, &evm_opts)?;

        let known_contracts = highlevel_known_contracts
            .iter()
            .map(|(id, c)| {
                (
                    id.clone(),
                    (
                        c.abi.clone(),
                        c.deployed_bytecode.clone().into_bytes().expect("not bytecode").to_vec(),
                    ),
                )
            })
            .collect::<BTreeMap<ArtifactId, (Abi, Vec<u8>)>>();

        let CompactContractBytecode { abi, bytecode, .. } = contract;
        let abi = abi.expect("no ABI for contract");
        let bytecode = bytecode.expect("no bytecode for contract").object.into_bytes().unwrap();
        let needs_setup = abi.functions().any(|func| func.name == "setUp");

        let env = evm_opts.evm_env();
        // the db backend that serves all the data
        let db = Backend::new(utils::get_fork(&evm_opts, &config.rpc_storage_caching), &env);

        let mut builder = ExecutorBuilder::new()
            .with_cheatcodes(evm_opts.ffi)
            .with_config(env)
            .with_spec(crate::utils::evm_spec(&config.evm_version))
            .with_gas_limit(evm_opts.gas_limit());

        if verbosity >= 3 {
            builder = builder.with_tracing();
        }

        let mut result = {
            let mut runner =
                Runner::new(builder.build(db), evm_opts.initial_balance, evm_opts.sender);
            let (address, mut result) =
                runner.setup(&predeploy_libraries, bytecode, needs_setup)?;

            let ScriptResult {
                success, gas, logs, traces, debug: run_debug, labeled_addresses, transactions, ..
            } = runner.script(
                address,
                if let Some(calldata) = self.sig.strip_prefix("0x") {
                    hex::decode(calldata)?.into()
                } else {
                    encode_args(&IntoFunction::into(self.sig), &self.args)?.into()
                },
            )?;

            result.success &= success;

            result.gas = gas;
            result.logs.extend(logs);
            result.traces.extend(traces);
            result.debug = run_debug;
            result.labeled_addresses.extend(labeled_addresses);
            match (&mut result.transactions, transactions) {
                (Some(txs), Some(new_txs)) => { txs.extend(new_txs); },
                (None, Some(new_txs)) => { result.transactions = Some(new_txs); },
                _ => {}
            }

            result
        };

        // Identify addresses in each trace
        let local_identifier = LocalTraceIdentifier::new(&known_contracts);
        let mut decoder = CallTraceDecoder::new_with_labels(result.labeled_addresses.clone());
        for (_, trace) in &mut result.traces {
            decoder.identify(trace, &local_identifier);
        }

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

        println!("Gas used: {}", result.gas);
        println!("== Logs ==");
        let console_logs = decode_console_logs(&result.logs);
        if !console_logs.is_empty() {
            for log in console_logs {
                println!("  {}", log);
            }
        }

        println!("Transactions: {:#?}", result.transactions);
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
    pub highlevel_known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    pub sources: BTreeMap<u32, String>,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
}

impl ScriptArgs {
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(&self, config: &Config, evm_opts: &EvmOpts) -> eyre::Result<BuildOutput> {
        let target_contract = dunce::canonicalize(&self.path)?;
        let project = config.ephemeral_no_artifacts_project()?;
        let output = compile_files(&project, vec![target_contract])?;

        let (contracts, sources) = output.into_artifacts_with_sources();
        let contracts: BTreeMap<ArtifactId, CompactContractBytecode> =
            contracts.into_iter().map(|(id, artifact)| (id, artifact.into())).collect();

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
            contracts,
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
                    id,
                    extra,
                    dependencies,
                } = post_link_input;

                // if it's the target contract, grab the info
                if extra.no_target_name {
                    if id.source == std::path::Path::new(&extra.target_fname) {
                        if extra.matched {
                            eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `-t ContractName`")
                        }
                        *extra.dependencies = dependencies;
                        *extra.contract = contract.clone();
                        extra.matched = true;
                    }
                } else {
                    let split: Vec<&str> = extra.target_fname.split(':').collect();
                    let path = std::path::Path::new(split[0]);
                    let name = split[1];
                    if path == id.source && name == id.name {
                        *extra.dependencies = dependencies;
                        *extra.contract = contract.clone();
                        extra.matched = true;
                    }
                }

                let tc: ContractBytecode = contract.into();
                highlevel_known_contracts.insert(id, tc.unwrap());
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

struct ScriptResult {
    pub success: bool,
    pub logs: Vec<RawLog>,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: Option<Vec<DebugArena>>,
    pub gas: u64,
    pub labeled_addresses: BTreeMap<Address, String>,
    pub transactions: Option<VecDeque<TypedTransaction>>,
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
    ) -> eyre::Result<(Address, ScriptResult)> {
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
                    gas,
                    transactions,
                    ..
                }) |
                Err(EvmError::Execution {
                    reverted,
                    traces: setup_traces,
                    labels,
                    logs: setup_logs,
                    debug,
                    gas,
                    transactions,
                    ..
                }) => {
                    traces
                        .extend(setup_traces.map(|traces| (TraceKind::Setup, traces)).into_iter());
                    logs.extend_from_slice(&setup_logs);

                    (
                        address,
                        ScriptResult {
                            logs,
                            traces,
                            labeled_addresses: labels,
                            success: !reverted,
                            debug: vec![constructor_debug, debug].into_iter().collect(),
                            gas,
                            transactions,
                        },
                    )
                }
                Err(e) => return Err(e.into()),
            }
        } else {
            (
                address,
                ScriptResult {
                    logs,
                    traces,
                    success: true,
                    debug: vec![constructor_debug].into_iter().collect(),
                    gas: 0,
                    labeled_addresses: Default::default(),
                    transactions: None,
                },
            )
        })
    }

    pub fn script(&mut self, address: Address, calldata: Bytes) -> eyre::Result<ScriptResult> {
        let RawCallResult { reverted, gas, stipend, logs, traces, labels, debug, transactions, .. } =
            self.executor.call_raw(self.sender, address, calldata.0, 0.into())?;
        Ok(ScriptResult {
            success: !reverted,
            gas: gas.overflowing_sub(stipend).0,
            logs,
            traces: traces.map(|traces| vec![(TraceKind::Execution, traces)]).unwrap_or_default(),
            debug: vec![debug].into_iter().collect(),
            labeled_addresses: labels,
            transactions,
        })
    }
}
