//! Sequential Monad simulation owns its block cursor independently of local script execution.

use super::{PreSimulationState, RpcContexts, RpcSimulationContext, context_for_rpc};
use crate::{
    ScriptResult,
    runner::{GasSearch, ScriptRunner},
    simulate::FilledTransactionsState,
    transaction::ScriptTransactionBuilder,
};
use alloy_eips::eip7702::SignedAuthorization;
use alloy_network::Ethereum;
use alloy_primitives::{Address, Bytes, TxKind, U256, map::HashMap};
use eyre::{Result, WrapErr};
use foundry_evm::{
    backend::DatabaseExt,
    core::{
        FoundryBlock, FoundryTransaction,
        evm::{BlockContext, ChainFor, EvmEnvFor, MonadEvmNetwork, TxEnvFor},
    },
    executors::{DeployResult, EvmError},
    revm::{context::Transaction, context_interface::result::Output, interpreter::return_ok},
};
use futures::future::join_all;
use parking_lot::RwLock;
use std::sync::Arc;

struct MonadSimulation {
    runner: ScriptRunner<MonadEvmNetwork>,
    cursor: Option<BlockContext<MonadEvmNetwork>>,
}

impl MonadSimulation {
    fn new(runner: ScriptRunner<MonadEvmNetwork>) -> Result<Self> {
        let cursor = runner.executor.backend().block_context_for_synthetic_transaction()?;
        Ok(Self { runner, cursor })
    }

    fn context(&self, tx: &TxEnvFor<MonadEvmNetwork>) -> Result<ChainFor<MonadEvmNetwork>> {
        self.cursor.as_ref().map_or_else(
            || self.runner.executor.backend().chain_context_for_synthetic_transaction(tx),
            |cursor| Ok(cursor.next_transaction(tx)),
        )
    }

    fn record(&mut self, tx: TxEnvFor<MonadEvmNetwork>) {
        if let Some(cursor) = &mut self.cursor {
            cursor.record_transaction(tx);
        }
    }

    fn prepare_call(
        &self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        authorization_list: Option<Vec<SignedAuthorization>>,
    ) -> (EvmEnvFor<MonadEvmNetwork>, TxEnvFor<MonadEvmNetwork>) {
        let (env, mut tx) = self.runner.executor.prepare_call_env(from, to.into(), calldata, value);
        if let Some(authorization_list) = authorization_list {
            tx.set_signed_authorization(authorization_list);
            tx.set_tx_type(4);
        }
        (env, tx)
    }

    fn simulate(
        &mut self,
        from: Address,
        to: Option<Address>,
        calldata: Option<Bytes>,
        value: Option<U256>,
        authorization_list: Option<Vec<SignedAuthorization>>,
    ) -> Result<ScriptResult<Ethereum>> {
        let value = value.unwrap_or_default();
        let Some(to) = to else {
            let (env, tx) = self.runner.executor.prepare_call_env(
                from,
                TxKind::Create,
                calldata.expect("No data for create transaction"),
                value,
            );
            let context = self.context(&tx)?;
            let result = self
                .runner
                .executor
                .transact_with_env_and_context(env, tx, context)
                .map_err(EvmError::from)
                .and_then(|raw| {
                    // Record before result conversion: even a skip/revert has already committed.
                    self.record(raw.tx_env.clone());
                    let raw = raw.into_result(None)?;
                    let Some(Output::Create(_, Some(address))) = raw.out else {
                        panic!("Deployment succeeded, but no address was returned: {raw:#?}");
                    };
                    self.runner.executor.backend_mut().add_persistent_account(address);
                    Ok(DeployResult { raw, address })
                });
            return self.runner.deployment_result(result);
        };

        let calldata = calldata.unwrap_or_default();
        let (env, tx) =
            self.prepare_call(from, to, calldata.clone(), value, authorization_list.clone());
        let context = self.context(&tx)?;
        let result = self.runner.executor.call_with_env_and_context(env, tx, context)?;
        let mut gas_used = result.gas_used;
        if matches!(result.exit_reason, Some(return_ok!())) {
            let initial_limit = self.runner.executor.tx_env().gas_limit();
            let mut search = GasSearch::new(gas_used);
            while let Some(limit) = search.next_limit() {
                // Keep the existing gas-probe preparation and authorization behavior unchanged.
                self.runner.executor.tx_env_mut().set_gas_limit(limit);
                let (env, tx) = self.prepare_call(from, to, calldata.clone(), value, None);
                let context = self.context(&tx)?;
                let result = self.runner.executor.call_with_env_and_context(env, tx, context)?;
                search.record(limit, result.exit_reason);
            }
            gas_used = search.gas_used();
            self.runner.executor.tx_env_mut().set_gas_limit(initial_limit);
        }

        let (env, tx) = self.prepare_call(from, to, calldata, value, authorization_list);
        let context = self.context(&tx)?;
        let result = self.runner.executor.transact_with_env_and_context(env, tx, context)?;
        self.record(result.tx_env.clone());
        Ok(self.runner.call_result(result, gas_used))
    }

    fn advance_block(&mut self) {
        if let Some(cursor) = &mut self.cursor {
            cursor.advance_block();
        }
        let block = &mut self.runner.executor.evm_env_mut().block_env;
        block.set_number(block.number + U256::from(1));
    }
}

impl PreSimulationState<MonadEvmNetwork> {
    pub(crate) async fn fill_monad_metadata(
        self,
    ) -> Result<FilledTransactionsState<MonadEvmNetwork>> {
        if self.args.skip_simulation {
            return self.fill_without_simulation().await;
        }

        let mut contexts = HashMap::default();
        for (rpc, context) in self.build_runners().await? {
            contexts.insert(
                rpc,
                RpcSimulationContext {
                    runner: RwLock::new(MonadSimulation::new(context.runner.into_inner())?),
                    decoder: context.decoder,
                },
            );
        }
        let contexts = Arc::new(contexts);
        let transactions =
            self.transaction_metadata(&RpcContexts::Simulation(Arc::clone(&contexts)))?;
        let futs = transactions
            .into_iter()
            .map(|mut transaction| async {
                let rpc = transaction.rpc.clone();
                let context = context_for_rpc(&contexts, &rpc);
                let mut simulation = context.runner.write();
                let tx = transaction.tx_mut();
                let to = tx.to();
                let result = simulation
                    .simulate(
                        tx.from()
                            .expect("transaction doesn't have a `from` address at execution time"),
                        to,
                        tx.input().cloned(),
                        tx.value(),
                        tx.authorization_list(),
                    )
                    .wrap_err("Internal EVM error during simulation")?;
                if !result.success {
                    return Ok((rpc, None, false, result.traces));
                }
                if self.args.slow {
                    simulation.advance_block();
                }
                let is_noop = if let Some(to) = to {
                    simulation.runner.executor.is_empty_code(to)?
                        && tx.value().unwrap_or_default().is_zero()
                } else {
                    false
                };
                let transaction = ScriptTransactionBuilder::from(transaction)
                    .with_execution_result(
                        &result,
                        self.args.gas_estimate_multiplier,
                        &self.build_data,
                    )
                    .build();
                eyre::Ok((rpc, Some(transaction), is_noop, result.traces))
            })
            .collect::<Vec<_>>();
        self.show_simulation_header()?;
        let transactions = self.collect_simulation_results(join_all(futs).await, &contexts).await?;
        Ok(self.into_filled(transactions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;
    use foundry_evm::{backend::Backend, executors::ExecutorBuilder, opts::EvmOpts};
    use foundry_evm_networks::NetworkConfigs;

    fn simulation() -> MonadSimulation {
        let mut env = EvmEnvFor::<MonadEvmNetwork>::default();
        // Match the simulation environment produced by EvmOpts.
        env.cfg_env.disable_nonce_check = true;
        let executor = ExecutorBuilder::<MonadEvmNetwork>::new().gas_limit(1 << 20).build(
            env,
            TxEnvFor::<MonadEvmNetwork>::default(),
            Backend::spawn(None).unwrap(),
            NetworkConfigs::with_monad(),
        );
        let runner = ScriptRunner::new(executor, EvmOpts::default());
        let mut simulation = MonadSimulation::new(runner).unwrap();
        assert!(simulation.cursor.is_none());
        simulation.cursor = Some(BlockContext::new(Vec::new(), Vec::new(), Vec::new()));
        simulation
    }

    fn assert_transaction_count(simulation: &MonadSimulation, count: usize) {
        let cursor = simulation.cursor.as_ref().unwrap();
        assert!(cursor.clone().before_transaction(count).is_ok());
        assert!(cursor.clone().before_transaction(count + 1).is_err());
    }

    #[test]
    fn calls_and_gas_probes_record_only_committed_transactions() {
        let other = simulation();
        let mut simulation = simulation();
        let sender = Address::with_last_byte(0x42);
        let recipient = Address::with_last_byte(0x43);
        simulation.runner.executor.set_balance(sender, U256::MAX).unwrap();
        let result = simulation.simulate(sender, Some(recipient), None, None, None).unwrap();
        assert!(result.success);
        assert_transaction_count(&simulation, 1);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 1);

        simulation.advance_block();
        assert_transaction_count(&simulation, 0);
        assert_eq!(simulation.runner.executor.evm_env().block_env.number, U256::from(1));
        assert_transaction_count(&other, 0);
    }

    #[test]
    fn deployment_and_reverted_call_both_advance_the_cursor() {
        let mut simulation = simulation();
        let sender = Address::with_last_byte(0x42);
        simulation.runner.executor.set_balance(sender, U256::MAX).unwrap();
        // Deploy runtime code that always reverts.
        let initcode = Bytes::from_static(&hex!("6005600c60003960056000f360006000fd"));
        let deployment = simulation.simulate(sender, None, Some(initcode), None, None).unwrap();
        assert!(deployment.success);
        assert_transaction_count(&simulation, 1);
        let address = deployment.address.unwrap();
        assert!(!simulation.runner.executor.is_empty_code(address).unwrap());
        let result = simulation.simulate(sender, Some(address), None, None, None).unwrap();
        assert!(!result.success);
        assert_transaction_count(&simulation, 2);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 2);
    }

    #[test]
    fn validation_failure_does_not_advance_the_cursor() {
        let mut simulation = simulation();
        simulation.runner.executor.set_gas_limit(1);
        let sender = Address::with_last_byte(0x42);
        let result =
            simulation.simulate(sender, Some(Address::with_last_byte(0x43)), None, None, None);
        assert!(result.is_err());
        assert_transaction_count(&simulation, 0);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 0);
    }

    #[test]
    fn authorization_preparation_preserves_explicit_empty_list() {
        let simulation = simulation();
        let (_, tx) = simulation.prepare_call(
            Address::with_last_byte(0x42),
            Address::with_last_byte(0x43),
            Bytes::new(),
            U256::ZERO,
            Some(Vec::new()),
        );
        assert_eq!(tx.tx_type(), 4);
        assert_transaction_count(&simulation, 0);
    }
}
