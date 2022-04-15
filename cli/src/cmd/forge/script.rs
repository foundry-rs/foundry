use crate::{
    cmd::{forge::build::BuildArgs, Cmd},
    compile,
    opts::{evm::EvmArgs, Wallet},
    utils,
};
use ansi_term::Colour;
use clap::{Parser, ValueHint};
use ethers::{
    abi::{Abi, RawLog},
    prelude::ArtifactId,
    solc::artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
    types::{
        transaction::eip2718::TypedTransaction, Address, Bytes, NameOrAddress, TransactionRequest,
        U256,
    },
};
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
use std::{
    collections::{BTreeMap, VecDeque},
    io::Write,
    path::PathBuf,
};

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

    #[clap(long, help = "execute the transactions")]
    pub execute: bool,

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
            target,
            mut contract,
            mut highlevel_known_contracts,
            mut predeploy_libraries,
            known_contracts: default_known_contracts,
        } = self.build(&config, &evm_opts, U256::one())?;

        let mut known_contracts = highlevel_known_contracts
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

        // execute once with default sender
        let mut result = self.execute(contract, &evm_opts, None, &predeploy_libraries, &config)?;

        let mut new_sender = None;
        if let Some(ref txs) = result.transactions {
            // If we did any linking with assumed predeploys, we cant support multiple deployers.
            //
            // If we didn't do any linking with predeploys, then none of the contracts will change
            // their code based on the sender, so we can skip relinking + reexecuting.
            if !predeploy_libraries.is_empty() {
                for tx in txs.iter() {
                    match tx {
                        TypedTransaction::Legacy(tx) => {
                            if tx.to.is_none() {
                                let sender = tx.from.expect("no sender");
                                if let Some(ns) = new_sender {
                                    if sender != ns {
                                        panic!("Currently, only 1 contract deployer is possible per public function. This limitation may be lifted in the future but for safety/simplicity this is currently disallowed. Split deployment into more functions & run separately.")
                                    }
                                } else if sender != evm_opts.sender {
                                    new_sender = Some(sender);
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }

        // reexecute with the correct deployer after relinking contracts
        if let Some(new_sender) = new_sender {
            // if we had a new sender that requires relinking, we need to
            // get the nonce mainnet for accurate addresses for predeploy libs
            let nonce = if let Some(ref fork_url) = evm_opts.fork_url {
                foundry_utils::next_nonce(new_sender, fork_url, None)?
            } else {
                U256::one()
            };
            // relink with new sender
            let BuildOutput {
                target: _,
                contract: c2,
                highlevel_known_contracts: hkc,
                predeploy_libraries: pl,
                known_contracts: _default_known_contracts,
            } = self.link(default_known_contracts, new_sender, nonce)?;

            contract = c2;
            highlevel_known_contracts = hkc;
            predeploy_libraries = pl;

            known_contracts = highlevel_known_contracts
                .iter()
                .map(|(id, c)| {
                    (
                        id.clone(),
                        (
                            c.abi.clone(),
                            c.deployed_bytecode
                                .clone()
                                .into_bytes()
                                .expect("not bytecode")
                                .to_vec(),
                        ),
                    )
                })
                .collect::<BTreeMap<ArtifactId, (Abi, Vec<u8>)>>();

            let lib_deploy = predeploy_libraries
                .iter()
                .map(|bytes| {
                    TypedTransaction::Legacy(TransactionRequest {
                        from: Some(new_sender),
                        data: Some(bytes.clone()),
                        ..Default::default()
                    })
                })
                .collect();
            result.transactions = Some(lib_deploy);

            let result2 =
                self.execute(contract, &evm_opts, Some(new_sender), &predeploy_libraries, &config)?;

            result.success &= result2.success;

            result.gas = result2.gas;
            result.logs = result2.logs;
            result.traces.extend(result2.traces);
            result.debug = result2.debug;
            result.labeled_addresses.extend(result2.labeled_addresses);
            match (&mut result.transactions, result2.transactions) {
                (Some(txs), Some(new_txs)) => {
                    txs.extend(new_txs);
                }
                (None, Some(new_txs)) => {
                    result.transactions = Some(new_txs);
                }
                _ => {}
            }
        }

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
                println!("Full Script Traces:");
                for (kind, trace) in &mut result.traces {
                    let should_include = match kind {
                        TraceKind::Setup => (verbosity >= 5) || (verbosity == 4 && !result.success),
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
            println!("{}", Colour::Green.paint("Dry running script was successfully."));
        } else {
            println!("{}", Colour::Red.paint("Dry running script failed."));
        }

        println!("Gas used: {}", result.gas);
        println!("== Logs ==");
        let console_logs = decode_console_logs(&result.logs);
        if !console_logs.is_empty() {
            for log in console_logs {
                println!("  {}", log);
            }
        }

        let tx_json = serde_json::to_string_pretty(&result.transactions).expect("Bad serializing");
        println!("\nGenerated Transactions:\n\n{}", tx_json);

        let mut out = config.out.clone();
        let target_fname = target.source.file_name().expect("No file name");
        out.push(target_fname);
        out.push("scripted_transactions");
        std::fs::create_dir_all(out.clone())?;
        out.push(self.sig.clone() + ".json");
        let mut file = std::fs::File::create(out.clone())?;
        file.write_all(tx_json.as_bytes())?;

        println!(
            "\nTransactions written to: {}\n",
            out.to_str().expect(
                "Couldn't convert path to string. Transactions were written to file though."
            )
        );

        println!("==========================");
        println!("Simulated On-chain Traces:\n");
        if let Some(txs) = result.transactions {
            self.execute_transactions(txs, &evm_opts, &config, &mut decoder);
        } else {
            panic!("No onchain transactions generated in script");
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
    target_id: Option<ArtifactId>,
}

pub struct BuildOutput {
    pub target: ArtifactId,
    pub contract: CompactContractBytecode,
    pub known_contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
    pub highlevel_known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    pub predeploy_libraries: Vec<ethers::types::Bytes>,
}

impl ScriptArgs {
    /// Compiles the file with auto-detection and compiler params.
    pub fn build(
        &self,
        config: &Config,
        evm_opts: &EvmOpts,
        nonce: U256,
    ) -> eyre::Result<BuildOutput> {
        let target_contract = dunce::canonicalize(&self.path)?;
        let project = config.ephemeral_no_artifacts_project()?;
        let output = compile::compile_files(&project, vec![target_contract])?;

        let (contracts, _sources) = output.into_artifacts_with_sources();
        let contracts: BTreeMap<ArtifactId, CompactContractBytecode> =
            contracts.into_iter().map(|(id, artifact)| (id, artifact.into())).collect();
        self.link(contracts, evm_opts.sender, nonce)
    }

    fn execute(
        &self,
        contract: CompactContractBytecode,
        evm_opts: &EvmOpts,
        sender: Option<Address>,
        predeploy_libraries: &[ethers::types::Bytes],
        config: &Config,
    ) -> eyre::Result<ScriptResult> {
        let CompactContractBytecode { abi, bytecode, .. } = contract;
        let abi = abi.expect("no ABI for contract");
        let bytecode = bytecode.expect("no bytecode for contract").object.into_bytes().unwrap();
        let needs_setup = abi.functions().any(|func| func.name == "setUp");

        let env = evm_opts.evm_env();
        // the db backend that serves all the data
        let db = Backend::new(utils::get_fork(evm_opts, &config.rpc_storage_caching), &env);

        let mut builder = ExecutorBuilder::new()
            .with_cheatcodes(evm_opts.ffi)
            .with_config(env)
            .with_spec(crate::utils::evm_spec(&config.evm_version))
            .with_gas_limit(evm_opts.gas_limit());

        if evm_opts.verbosity >= 3 {
            builder = builder.with_tracing();
        }

        let mut runner = Runner::new(
            builder.build(db),
            evm_opts.initial_balance,
            sender.unwrap_or(evm_opts.sender),
        );
        let (address, mut result) = runner.setup(predeploy_libraries, bytecode, needs_setup)?;

        let ScriptResult {
            success,
            gas,
            logs,
            traces,
            debug: run_debug,
            labeled_addresses,
            transactions,
            ..
        } = runner.script(
            address,
            if let Some(calldata) = self.sig.strip_prefix("0x") {
                hex::decode(calldata)?.into()
            } else {
                encode_args(&IntoFunction::into(self.sig.clone()), &self.args)?.into()
            },
        )?;

        result.success &= success;

        result.gas = gas;
        result.logs.extend(logs);
        result.traces.extend(traces);
        result.debug = run_debug;
        result.labeled_addresses.extend(labeled_addresses);
        match (&mut result.transactions, transactions) {
            (Some(txs), Some(new_txs)) => {
                txs.extend(new_txs);
            }
            (None, Some(new_txs)) => {
                result.transactions = Some(new_txs);
            }
            _ => {}
        }

        Ok(result)
    }

    fn execute_transactions(
        &self,
        transactions: VecDeque<TypedTransaction>,
        evm_opts: &EvmOpts,
        config: &Config,
        decoder: &mut CallTraceDecoder,
    ) {
        let env = evm_opts.evm_env();
        // the db backend that serves all the data
        let db = Backend::new(utils::get_fork(evm_opts, &config.rpc_storage_caching), &env);

        let mut builder = ExecutorBuilder::new()
            .with_cheatcodes(evm_opts.ffi)
            .with_config(env)
            .with_spec(crate::utils::evm_spec(&config.evm_version))
            .with_gas_limit(evm_opts.gas_limit());

        if evm_opts.verbosity >= 3 {
            builder = builder.with_tracing();
        }

        let mut runner = Runner::new(builder.build(db), evm_opts.initial_balance, evm_opts.sender);

        transactions
            .into_iter()
            .map(|tx| match tx {
                TypedTransaction::Legacy(tx) => (tx.from, tx.to, tx.data, tx.value),
                _ => unreachable!(),
            })
            .map(|(from, to, data, value)| {
                runner
                    .sim(
                        from.expect("Transaction doesn't have a `from` address at execution time"),
                        to,
                        data,
                        value,
                    )
                    .expect("Internal EVM error")
            })
            .for_each(|mut result| {
                for (_kind, trace) in &mut result.traces {
                    decoder.decode(trace);
                    println!("{}", trace);
                }
            });
    }

    pub fn link(
        &self,
        contracts: BTreeMap<ArtifactId, CompactContractBytecode>,
        sender: Address,
        nonce: U256,
    ) -> eyre::Result<BuildOutput> {
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

        let mut extra_info = ExtraLinkingInfo {
            no_target_name,
            target_fname,
            contract: &mut contract,
            dependencies: &mut run_dependencies,
            matched: false,
            target_id: None,
        };
        foundry_utils::link_with_nonce(
            contracts.clone(),
            &mut highlevel_known_contracts,
            sender,
            nonce,
            &mut extra_info,
            |file, key| (format!("{}.json:{}", key, key), file, key),
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
                            eyre::bail!("Multiple contracts in the target path. Please specify the contract name with `--tc ContractName`")
                        }
                        *extra.dependencies = dependencies;
                        *extra.contract = contract.clone();
                        extra.matched = true;
                        extra.target_id = Some(id.clone());
                    }
                } else {
                    let split: Vec<&str> = extra.target_fname.split(':').collect();
                    let path = std::path::Path::new(split[0]);
                    let name = split[1];
                    if path == id.source && name == id.name {
                        *extra.dependencies = dependencies;
                        *extra.contract = contract.clone();
                        extra.matched = true;
                        extra.target_id = Some(id.clone());
                    }
                }

                let tc: ContractBytecode = contract.into();
                highlevel_known_contracts.insert(id, tc.unwrap());
                Ok(())
            },
        )?;

        Ok(BuildOutput {
            target: extra_info.target_id.expect("Target not found?"),
            contract,
            known_contracts: contracts,
            highlevel_known_contracts,
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
        let RawCallResult {
            reverted, gas, stipend, logs, traces, labels, debug, transactions, ..
        } = self.executor.call_raw(self.sender, address, calldata.0, 0.into())?;
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

    pub fn sim(
        &mut self,
        from: Address,
        to: Option<NameOrAddress>,
        calldata: Option<Bytes>,
        value: Option<U256>,
    ) -> eyre::Result<ScriptResult> {
        if let Some(NameOrAddress::Address(to)) = to {
            let RawCallResult {
                reverted,
                gas,
                stipend,
                logs,
                traces,
                labels,
                debug,
                transactions,
                ..
            } = self.executor.call_raw(
                from,
                to,
                calldata.unwrap_or_default().0,
                value.unwrap_or(U256::zero()),
            )?;
            Ok(ScriptResult {
                success: !reverted,
                gas: gas.overflowing_sub(stipend).0,
                logs,
                traces: traces
                    .map(|traces| vec![(TraceKind::Execution, traces)])
                    .unwrap_or_default(),
                debug: vec![debug].into_iter().collect(),
                labeled_addresses: labels,
                transactions,
            })
        } else if to.is_none() {
            let DeployResult { address: _, gas, logs, traces, debug } = self.executor.deploy(
                from,
                calldata.expect("No data for create transaction").0,
                value.unwrap_or(U256::zero()),
            )?;
            Ok(ScriptResult {
                success: true,
                gas,
                logs,
                traces: traces
                    .map(|traces| vec![(TraceKind::Execution, traces)])
                    .unwrap_or_default(),
                debug: vec![debug].into_iter().collect(),
                labeled_addresses: Default::default(),
                transactions: Default::default(),
            })
        } else {
            panic!("ens not supported");
        }
    }
}
