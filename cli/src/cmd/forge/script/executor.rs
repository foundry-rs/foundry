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
use std::collections::VecDeque;
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
        transactions: VecDeque<TypedTransaction>,
        script_config: &mut ScriptConfig,
        decoder: &mut CallTraceDecoder,
        contracts: &BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    ) -> eyre::Result<VecDeque<TransactionWithMetadata>> {
        let mut runner = self
            .prepare_runner(script_config, script_config.evm_opts.sender, SimulationStage::OnChain)
            .await;
        let mut failed = false;

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

        let mut final_txs = VecDeque::new();
        for tx in transactions {
            match tx {
                TypedTransaction::Legacy(mut tx) => {
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

                    // We inflate the gas used by the user specified percentage
                    tx.gas = Some(U256::from(result.gas * self.gas_estimate_multiplier / 100));

                    if !result.success {
                        failed = true;
                    }

                    if script_config.evm_opts.verbosity > 3 {
                        for (_kind, trace) in &mut result.traces {
                            decoder.decode(trace).await;
                            println!("{}", trace);
                        }
                    }

                    final_txs.push_back(TransactionWithMetadata::new(
                        tx.into(),
                        &result,
                        &address_to_abi,
                        decoder,
                    )?);
                }
                _ => unreachable!(),
            }
        }

        if failed {
            eyre::bail!("Simulated execution failed")
        } else {
            Ok(final_txs)
        }
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

        // The db backend that serves all the data.
        let db = script_config.backend.clone().unwrap_or_else(|| {
            let backend =
                Backend::spawn(script_config.evm_opts.get_fork(&script_config.config, env.clone()));
            script_config.backend = Some(backend.clone());
            backend
        });

        let mut builder = ExecutorBuilder::default()
            .with_config(env)
            .with_spec(utils::evm_spec(&script_config.config.evm_version))
            .with_gas_limit(script_config.evm_opts.gas_limit())
            // We need it enabled to decode contract names: local or external.
            .set_tracing(true);

        if let SimulationStage::Local = stage {
            builder = builder
                .set_debugger(self.debug)
                .with_cheatcodes(CheatsConfig::new(&script_config.config, &script_config.evm_opts));
        }

        ScriptRunner::new(builder.build(db), script_config.evm_opts.initial_balance, sender)
    }
}
