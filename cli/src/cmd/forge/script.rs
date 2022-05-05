use crate::{
    cmd::{forge::build::BuildArgs, Cmd, ScriptSequence},
    compile,
    opts::MultiWallet,
    utils,
};
use clap::{Parser, ValueHint};
use ethers::{
    abi::{Abi, RawLog},
    prelude::{ArtifactId, Provider, SignerMiddleware},
    providers::Middleware,
    signers::Signer,
    solc::artifacts::{CompactContractBytecode, ContractBytecode, ContractBytecodeSome},
    types::{
        transaction::eip2718::TypedTransaction, Address, Bytes, Chain, Eip1559TransactionRequest,
        NameOrAddress, TransactionReceipt, TransactionRequest, U256,
    },
};
use forge::{
    debug::DebugArena,
    decode::decode_console_logs,
    executor::{
        builder::Backend, opts::EvmOpts, CallResult, DatabaseRef, DeployResult, EvmError, Executor,
        ExecutorBuilder, RawCallResult,
    },
    trace::{
        identifier::LocalTraceIdentifier, CallTraceArena, CallTraceDecoder,
        CallTraceDecoderBuilder, TraceKind,
    },
    CALLER,
};
use foundry_common::evm::EvmArgs;
use foundry_config::{figment::Figment, Config};
use foundry_utils::{encode_args, IntoFunction, PostLinkInput, RuntimeOrHandle};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};
use yansi::Paint;

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
    pub wallets: MultiWallet,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,

    #[clap(
        long,
        help = "resumes previous transaction batch. DOES NOT simulate execution. respects nonce constraint "
    )]
    pub resume: bool,

    #[clap(
        long,
        help = "resumes previous transactions batch. DOES NOT simulate execution. DOES NOT respect nonce constraint"
    )]
    pub force_resume: bool,
}

impl Cmd for ScriptArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let figment: Figment = From::from(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let verbosity = evm_opts.verbosity;
        let config = Config::from_provider(figment).sanitized();

        let fork_url = evm_opts.fork_url.as_ref().expect("No url provided.");
        let nonce = foundry_utils::next_nonce(evm_opts.sender, fork_url, None)?;

        let BuildOutput {
            target, contract, highlevel_known_contracts, predeploy_libraries, ..
        } = self.build(&config, &evm_opts, nonce)?;

        if !self.force_resume && !self.resume {
            let known_contracts = highlevel_known_contracts
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

            let mut result = self.execute(
                contract,
                &evm_opts,
                Some(evm_opts.sender),
                &predeploy_libraries,
                &config,
            )?;

            let mut lib_deploy: VecDeque<TypedTransaction> = predeploy_libraries
                .iter()
                .enumerate()
                .map(|(i, bytes)| {
                    TypedTransaction::Legacy(TransactionRequest {
                        from: Some(evm_opts.sender),
                        data: Some(bytes.clone()),
                        nonce: Some(nonce + i),
                        ..Default::default()
                    })
                })
                .collect();

            // prepend predeploy libraries
            if let Some(txs) = &mut result.transactions {
                txs.iter().for_each(|tx| {
                    lib_deploy.push_back(TypedTransaction::Legacy(into_legacy(tx.clone())));
                });
                *txs = lib_deploy;
            }

            // Identify addresses in each trace
            let local_identifier = LocalTraceIdentifier::new(&known_contracts);
            let mut decoder = CallTraceDecoderBuilder::new()
                .with_labels(result.labeled_addresses.clone())
                .build();
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
                println!("{}", Paint::green("Dry running script was successful."));
            } else {
                println!("{}", Paint::red("Dry running script failed."));
            }

            println!("Gas used: {}", result.gas);
            println!("== Logs ==");
            let console_logs = decode_console_logs(&result.logs);
            if !console_logs.is_empty() {
                for log in console_logs {
                    println!("  {}", log);
                }
            }

            println!("==========================");
            println!("Simulated On-chain Traces:\n");
            if let Some(txs) = result.transactions {
                if let Ok(gas_filled_txs) =
                    self.execute_transactions(txs, &evm_opts, &config, &mut decoder)
                {
                    println!("\n\n==========================");
                    if !result.success {
                        panic!("\nSIMULATION FAILED");
                    } else {
                        let txs = gas_filled_txs;
                        let mut deployment_sequence =
                            ScriptSequence::new(txs, &self.sig, &target, &config.out)?;
                        deployment_sequence.save()?;

                        if self.execute {
                            self.send_transactions(&mut deployment_sequence)?;
                        } else {
                            println!("\nSIMULATION COMPLETE. To send these transaction onchain, add `--execute` & wallet configuration(s) to the previously ran command. See forge script --help for more.");
                        }
                    }
                } else {
                    panic!("One or more transactions failed when simulating the on-chain version. Check the trace via rerunning with `-vvv`")
                }
            } else {
                panic!("No onchain transactions generated in script");
            }
        } else {
            let mut deployment_sequence = ScriptSequence::load(&self.sig, &target, &config.out)?;
            self.send_transactions(&mut deployment_sequence)?;
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

fn set_chain_id(tx: &mut TypedTransaction, chain_id: u64) {
    match tx {
        TypedTransaction::Legacy(tx) => tx.chain_id = Some(chain_id.into()),
        TypedTransaction::Eip1559(tx) => tx.chain_id = Some(chain_id.into()),
        _ => panic!("Wrong transaction type for expected output"),
    }
}

fn into_legacy(tx: TypedTransaction) -> TransactionRequest {
    match tx {
        TypedTransaction::Legacy(tx) => tx,
        _ => panic!("Wrong transaction type for expected output"),
    }
}

fn into_legacy_ref(tx: &TypedTransaction) -> &TransactionRequest {
    match tx {
        TypedTransaction::Legacy(ref tx) => tx,
        _ => panic!("Wrong transaction type for expected output"),
    }
}

fn into_1559(tx: TypedTransaction) -> Eip1559TransactionRequest {
    match tx {
        TypedTransaction::Legacy(tx) => Eip1559TransactionRequest {
            from: tx.from,
            to: tx.to,
            value: tx.value,
            data: tx.data,
            nonce: tx.nonce,
            ..Default::default()
        },
        TypedTransaction::Eip1559(tx) => tx,
        _ => panic!("Wrong transaction type for expected output"),
    }
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

        let runtime = RuntimeOrHandle::new();
        let env = runtime.block_on(evm_opts.evm_env());
        // the db backend that serves all the data
        let db = runtime
            .block_on(Backend::new(utils::get_fork(evm_opts, &config.rpc_storage_caching), &env));

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
    ) -> eyre::Result<VecDeque<TypedTransaction>> {
        let runtime = RuntimeOrHandle::new();
        let env = runtime.block_on(evm_opts.evm_env());
        // the db backend that serves all the data
        let db = runtime
            .block_on(Backend::new(utils::get_fork(evm_opts, &config.rpc_storage_caching), &env));

        let mut builder = ExecutorBuilder::new()
            .with_cheatcodes(evm_opts.ffi)
            .with_config(env)
            .with_spec(crate::utils::evm_spec(&config.evm_version))
            .with_gas_limit(evm_opts.gas_limit());

        if evm_opts.verbosity >= 3 {
            builder = builder.with_tracing();
        }

        let mut runner = Runner::new(builder.build(db), evm_opts.initial_balance, evm_opts.sender);
        let mut failed = false;
        let mut sum_gas = 0;
        let mut final_txs = transactions.clone();
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
            .enumerate()
            .for_each(|(i, mut result)| {
                match &mut final_txs[i] {
                    TypedTransaction::Legacy(tx) => tx.gas = Some(U256::from(result.gas * 12 / 10)),
                    _ => unreachable!(),
                }

                sum_gas += result.gas;
                if !result.success {
                    failed = true;
                }
                for (_kind, trace) in &mut result.traces {
                    decoder.decode(trace);
                    println!("{}", trace);
                }
            });

        println!("Estimated total gas used for script: {}", sum_gas);
        if failed {
            Err(eyre::Report::msg("Simulated execution failed"))
        } else {
            Ok(final_txs)
        }
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

    fn send_transactions(&self, deployment_sequence: &mut ScriptSequence) -> eyre::Result<()> {
        // The user wants to actually send the transactions
        let mut local_wallets = vec![];
        if let Some(wallets) = self.wallets.private_keys()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet));
        }

        if let Some(wallets) = self.wallets.interactives()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet));
        }

        if let Some(wallets) = self.wallets.mnemonics()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet));
        }

        if let Some(wallets) = self.wallets.keystores()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet));
        }

        // TODO: Add trezor and ledger support (supported in multiwallet, just need to
        // add derivation + SignerMiddleware creation logic)
        // foundry/cli/src/opts/mod.rs:110
        if local_wallets.is_empty() {
            panic!("Error accessing local wallet when trying to send onchain transaction, did you set a private key, mnemonic or keystore?")
        }

        let fork_url = self
            .evm_opts
            .fork_url
            .as_ref()
            .expect("No fork_url provided for onchain sending")
            .clone();
        let provider = Provider::try_from(&fork_url).expect("Bad fork_url provider");

        let rt = RuntimeOrHandle::new();
        let chain = rt.block_on(provider.get_chainid())?.as_u64();
        let is_legacy =
            self.legacy || Chain::try_from(chain).map(|x| Chain::is_legacy(&x)).unwrap_or_default();
        local_wallets =
            local_wallets.into_iter().map(|wallet| wallet.with_chain_id(chain)).collect();

        // in case of --force-resume, we forgive the first nonce disparity of each from
        let mut nonce_offset: BTreeMap<Address, U256> = BTreeMap::new();

        // Iterate through transactions, matching the `from` field with the associated
        // wallet. Then send the transaction. Panics if we find a unknown `from`
        deployment_sequence
            .clone()
            .transactions
            .range((deployment_sequence.index as usize)..)
            .map(|tx| {
                let from = into_legacy_ref(tx).from.expect("No from for onchain transaction!");
                if let Some(wallet) =
                    local_wallets.iter().find(|wallet| (**wallet).address() == from)
                {
                    let signer = SignerMiddleware::new(provider.clone(), wallet.clone());
                    Ok((tx.clone(), signer))
                } else {
                    Err(eyre::eyre!(format!(
                        "No associated wallet for `from` address: {:?}. Unlocked wallets: {:?}. \nMake sure you have loaded all the private keys including of the `--sender`.",
                        from,
                        local_wallets
                            .iter()
                            .map(|wallet| wallet.address())
                            .collect::<Vec<Address>>()
                    )))
                }
            })
            .for_each(|payload| {
                match payload {
                    Ok((tx, signer)) => {
                        let mut legacy_or_1559 = if is_legacy {
                            tx
                        } else {
                            TypedTransaction::Eip1559(into_1559(tx))
                        };
                        set_chain_id(&mut legacy_or_1559, chain);

                        let from = *legacy_or_1559.from().expect("no sender");
                        match foundry_utils::next_nonce(from, &fork_url, None) {
                            Ok(nonce) => {
                                let tx_nonce = *legacy_or_1559.nonce().expect("no nonce");
                                let offset = if self.force_resume {
                                    match nonce_offset.get(&from) {
                                        Some(offset) => *offset,
                                        None => {
                                            let offset = nonce - tx_nonce;
                                            nonce_offset.insert(from, offset);
                                            offset
                                        }
                                    }
                                } else {
                                    U256::from(0u32)
                                };

                                if nonce != tx_nonce + offset {
                                    deployment_sequence
                                        .save()
                                        .expect("not able to save deployment sequence");
                                    panic!("EOA nonce changed unexpectedly while sending transactions.");
                                } else if !offset.is_zero() {
                                    legacy_or_1559.set_nonce(tx_nonce + offset);
                                }
                            }
                            Err(_) => {
                                deployment_sequence.save().expect("not able to save deployment sequence");
                                panic!("Not able to query the EOA nonce.");
                            }
                        }

                        async fn send<T, U>(
                            signer: SignerMiddleware<T, U>,
                            legacy_or_1559: TypedTransaction,
                        ) -> eyre::Result<Option<TransactionReceipt>>
                        where
                            SignerMiddleware<T, U>: Middleware,
                        {
                            tracing::debug!("sending transaction: {:?}", legacy_or_1559);
                            match signer.send_transaction(legacy_or_1559, None).await {
                                Ok(pending) => pending.await.map_err(|e| eyre::eyre!(e)),
                                Err(e) => Err(eyre::eyre!(e.to_string())),
                            }
                        }

                        let receipt = match rt.block_on(send(signer, legacy_or_1559)) {
                            Ok(Some(res)) => {
                                let tx_str = serde_json::to_string_pretty(&res).expect("Bad serialization");
                                println!("{}", tx_str);
                                res
                            }

                            Ok(None) => {
                                // todo what if it has been actually sent
                                deployment_sequence.save().expect("not able to save deployment sequence");
                                panic!("Failed to get transaction receipt?")
                            }
                            Err(e) => {
                                deployment_sequence.save().expect("not able to save deployment sequence");
                                panic!("Aborting! A transaction failed to send: {:#?}", e)
                            }
                        };

                        deployment_sequence.add_receipt(receipt);
                        deployment_sequence.index += 1;
                    }
                    Err(e) => {
                        deployment_sequence.save().expect("not able to save deployment sequence");
                        panic!("{e}");
                    }
                }
            });

        deployment_sequence.save()?;

        println!("\n\n==========================");
        println!(
            "\nONCHAIN EXECUTION COMPLETE & SUCCESSFUL. Transaction receipts written to {:?}",
            deployment_sequence.path
        );
        Ok(())
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
        self.executor.set_balance(*CALLER, U256::MAX);

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
        } = self.executor.deploy(*CALLER, code.0, 0u32.into()).expect("couldn't deploy");
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
        } = self.executor.call_raw(*CALLER, address, calldata.0, 0.into())?;
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
            let RawCallResult { reverted, gas, logs, traces, labels, debug, transactions, .. } =
                self.executor.call_raw(
                    from,
                    to,
                    calldata.unwrap_or_default().0,
                    value.unwrap_or(U256::zero()),
                )?;
            Ok(ScriptResult {
                success: !reverted,
                gas,
                logs,
                traces: traces
                    .map(|mut traces| {
                        // Manually adjust gas for the trace to add back the stipend/real used gas
                        traces.arena[0].trace.gas_cost = gas;
                        vec![(TraceKind::Execution, traces)]
                    })
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
                    .map(|mut traces| {
                        // Manually adjust gas for the trace to add back the stipend/real used gas
                        traces.arena[0].trace.gas_cost = gas;
                        vec![(TraceKind::Execution, traces)]
                    })
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
