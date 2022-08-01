use super::*;
use crate::{
    cmd::{
        forge::script::{runner::SimulationStage, sequence::TransactionWithMetadata},
        needs_setup,
    },
    utils,
};
use ethers::{
    solc::artifacts::CompactContractBytecode,
    types::{transaction::eip2718::TypedTransaction, Address, U256},
};
use forge::{
    executor::{inspector::CheatsConfig, Backend, ExecutorBuilder},
    trace::CallTraceDecoder,
};
use futures::future::join_all;
use parking_lot::RwLock;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};
use tracing::trace;

impl ScriptArgs {
    /// Locally deploys and executes the contract method that will collect all broadcastable
    /// transactions.
    pub async fn execute(
        &self,
        script_config: &mut ScriptConfig,
        contract: CompactContractBytecode,
        sender: Address,
        predeploy_libraries: &[ethers::types::Bytes],
    ) -> eyre::Result<ScriptResult> {
        trace!("start executing script");
        let CompactContractBytecode { abi, bytecode, .. } = contract;

        let abi = abi.expect("no ABI for contract");
        let bytecode = bytecode.expect("no bytecode for contract").object.into_bytes().unwrap();

        let mut runner = self.prepare_runner(script_config, sender, SimulationStage::Local).await;
        let (address, mut result) = runner.setup(
            predeploy_libraries,
            bytecode,
            needs_setup(&abi),
            script_config.sender_nonce,
            self.broadcast,
            script_config.evm_opts.fork_url.is_none(),
        )?;

        let (func, calldata) = self.get_method_and_calldata(&abi)?;
        script_config.called_function = Some(func);

        let script_result = runner.script(address, calldata)?;

        result.success &= script_result.success;
        result.gas = script_result.gas;
        result.logs.extend(script_result.logs);
        result.traces.extend(script_result.traces);
        result.debug = script_result.debug;
        result.labeled_addresses.extend(script_result.labeled_addresses);
        result.returned = script_result.returned;

        match (&mut result.transactions, script_result.transactions) {
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

    /// Executes a list of transactions locally and persists their state. Returns the transactions
    /// and any CREATE2 contract addresses created.
    pub async fn execute_transactions(
        &self,
        transactions: VecDeque<BroadcastableTransaction>,
        script_config: &mut ScriptConfig,
        decoder: &mut CallTraceDecoder,
        contracts: &BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    ) -> eyre::Result<VecDeque<TransactionWithMetadata>> {
        let runners = Arc::new(
            self.build_runners(script_config, &transactions)
                .await
                .into_iter()
                .map(|(k, runner)| (k, Arc::new(RwLock::new(runner))))
                .collect::<HashMap<_, _>>(),
        );

        if script_config.evm_opts.verbosity > 3 {
            println!("==========================");
            println!("Simulated On-chain Traces:\n");
        }

        let address_to_abi: BTreeMap<Address, (String, &Abi)> = decoder
            .contracts
            .iter()
            .filter_map(|(addr, contract_id)| {
                let contract_name = utils::get_contract_name(contract_id);
                if let Some((_, (abi, _))) =
                    contracts.iter().find(|(artifact, _)| artifact.name == contract_name)
                {
                    return Some((*addr, (contract_name.to_string(), abi)))
                }
                None
            })
            .collect();

        let final_txs = Arc::new(RwLock::new(VecDeque::new()));

        // for transaction in transactions
        let futs = transactions
            .into_iter()
            .map(|transaction| async {
                let mut runner = runners
                    .get(transaction.rpc.as_ref().expect("to have been filled already."))
                    .expect("to have been built.")
                    .write();

                let mut failed = false;

                if let TypedTransaction::Legacy(mut tx) = transaction.transaction {
                    let mut result = runner
                        .simulate(
                            tx.from.expect(
                                "Transaction doesn't have a `from` address at execution time",
                            ),
                            tx.to.clone(),
                            tx.data.clone(),
                            tx.value,
                        )
                        .expect("Internal EVM error");

                    // Simulate mining the transaction if the user passes `--slow`.
                    if self.slow {
                        runner.executor.env_mut().block.number += U256::one();
                    }

                    // We inflate the gas used by the transaction by x1.3 since the estimation
                    // might be off
                    tx.gas = Some(U256::from(result.gas * 13 / 10));

                    if !result.success {
                        failed = true;
                    }

                    if script_config.evm_opts.verbosity > 3 {
                        for (_kind, trace) in &mut result.traces {
                            decoder.decode(trace).await;
                            println!("{}", trace);
                        }
                    }

                    final_txs.write().push_back(TransactionWithMetadata::new(
                        tx.into(),
                        transaction.rpc,
                        &result,
                        &address_to_abi,
                        decoder,
                    ));
                }
                failed
            })
            .collect::<Vec<_>>();

        for failed in join_all(futs).await {
            if failed {
                eyre::bail!("Simulated execution failed")
            }
        }
        Ok(Arc::try_unwrap(final_txs).expect("Only one ref").into_inner())
    }

    /// Build the multiple runners from different forks.
    async fn build_runners(
        &self,
        script_config: &mut ScriptConfig,
        transactions: &VecDeque<BroadcastableTransaction>,
    ) -> HashMap<String, ScriptRunner> {
        let runners = Arc::new(RwLock::new(HashMap::new()));
        let sender = script_config.evm_opts.sender;

        let unique_rpcs = transactions
            .iter()
            .map(|tx| tx.rpc.clone().unwrap_or_default())
            .collect::<HashSet<String>>();

        let futs = unique_rpcs
            .iter()
            .map(|rpc| async {
                let mut script_config = script_config.clone();
                script_config.evm_opts.fork_url = Some(rpc.clone());

                let runner =
                    self.prepare_runner(&mut script_config, sender, SimulationStage::OnChain).await;

                runners.write().insert(rpc.clone(), runner);
            })
            .collect::<Vec<_>>();

        join_all(futs).await;

        Arc::try_unwrap(runners).expect("Only one ref.").into_inner()
    }

    /// Creates the Runner that drives script execution
    async fn prepare_runner(
        &self,
        script_config: &mut ScriptConfig,
        sender: Address,
        stage: SimulationStage,
    ) -> ScriptRunner {
        trace!("preparing script runner");
        let env = script_config.evm_opts.evm_env().await;
        let url = script_config.evm_opts.fork_url.clone().unwrap_or_default();

        // TODO(joshie): somehow get fork backend from the local simulation run
        // The db backend that serves all the data.
        let db = match script_config.backend.get(&url) {
            Some(db) => db.clone(),
            None => {
                let backend = Backend::spawn(
                    script_config.evm_opts.get_fork(&script_config.config, env.clone()),
                );
                script_config.backend.insert(url.clone(), backend);
                script_config.backend.get(&url).unwrap().clone()
            }
        };

        let mut builder = ExecutorBuilder::default()
            .with_config(env)
            .with_spec(utils::evm_spec(&script_config.config.evm_version))
            .with_gas_limit(script_config.evm_opts.gas_limit())
            .set_tracing(script_config.evm_opts.verbosity >= 3 || self.debug);

        if let SimulationStage::Local = stage {
            builder = builder
                .set_debugger(self.debug)
                .with_cheatcodes(CheatsConfig::new(&script_config.config, &script_config.evm_opts));
        }

        ScriptRunner::new(builder.build(db), script_config.evm_opts.initial_balance, sender)
    }
}
