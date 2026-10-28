//! Sequential Monad simulation owns its block cursor independently of local script execution.

use super::{PreSimulationState, RpcSimulationContext};
use crate::{
    ScriptResult,
    runner::{GasSearch, ScriptRunner, needs_more_gas},
    simulate::FilledTransactionsState,
    transaction::ScriptTransactionBuilder,
};
use alloy_consensus::transaction::Recovered;
use alloy_eips::eip7702::SignedAuthorization;
use alloy_network::{AnyNetwork, Ethereum, TransactionBuilder};
use alloy_primitives::{Address, Bytes, TxKind, U256, map::HashMap};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use evm2::interpreter::InstrStop;
use eyre::{Result, WrapErr};
use foundry_common::{FoundryTransactionBuilder, TransactionMaybeSigned};
use foundry_evm::{
    core::evm::{
        BlockContext, ChainFor, EvmEnvFor, EvmFactoryFor, FoundryEvmFactory, FoundryEvmNetwork,
        MonadEvmNetwork, TxFor,
    },
    executors::{EvmError, RawCallResult},
};

struct MonadSimulation {
    runner: ScriptRunner<MonadEvmNetwork>,
    cursor: Option<BlockContext<MonadEvmNetwork>>,
}

impl MonadSimulation {
    /// Creates a simulation whose cursor starts in the block after the runner's fork block, if
    /// forked.
    async fn new(runner: ScriptRunner<MonadEvmNetwork>) -> Result<Self> {
        let backend = runner.executor.backend();
        let cursor = match (backend.fork(), backend.active_fork_url()) {
            (Some(fork), Some(url)) => {
                let provider = runner.evm_opts.fork_provider_with_url::<AnyNetwork>(&url)?;
                let block = provider
                    .get_block_by_hash(fork.hash())
                    .full()
                    .await?
                    .ok_or_else(|| eyre::eyre!("fork block {} not found", fork.hash()))?;
                let mut cursor = BlockContext::fetch(&provider, &block).await?;
                cursor.advance_block();
                Some(cursor)
            }
            _ => None,
        };
        Ok(Self { runner, cursor })
    }

    /// Returns the extension state for `tx` at the cursor's next position, if positioned.
    fn ext(
        &self,
        env: &EvmEnvFor<MonadEvmNetwork>,
        tx: &Recovered<TxFor<MonadEvmNetwork>>,
    ) -> Option<ChainFor<MonadEvmNetwork>> {
        self.cursor.as_ref().map(|cursor| {
            let mut ext = EvmFactoryFor::<MonadEvmNetwork>::default().evm_ext(env);
            cursor.prepare_next(&mut ext, tx);
            ext
        })
    }

    /// Executes `tx` without committing it.
    fn call(
        &self,
        env: EvmEnvFor<MonadEvmNetwork>,
        tx: TransactionRequest,
    ) -> Result<RawCallResult<MonadEvmNetwork>> {
        let tx = MonadEvmNetwork::tx_from_request(tx)?;
        self.runner.executor.call_tx(env, &tx, self.ext(&env, &tx))
    }

    /// Executes and commits `tx`, and records it at the cursor's position.
    ///
    /// Reverted transactions are committed and recorded too; transactions failing validation are
    /// neither.
    fn transact(
        &mut self,
        env: EvmEnvFor<MonadEvmNetwork>,
        tx: TransactionRequest,
    ) -> Result<RawCallResult<MonadEvmNetwork>> {
        let tx = MonadEvmNetwork::tx_from_request(tx)?;
        let result = self.runner.executor.transact_tx(env, &tx, self.ext(&env, &tx))?;
        if let Some(cursor) = &mut self.cursor {
            cursor.record_transaction(tx);
        }
        Ok(result)
    }

    fn prepare_call(
        &self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        authorization_list: Option<Vec<SignedAuthorization>>,
    ) -> (EvmEnvFor<MonadEvmNetwork>, TransactionRequest) {
        let (env, mut tx) = self.runner.executor.prepare_call_env(from, to.into(), calldata, value);
        if let Some(authorization_list) = authorization_list {
            tx.set_authorization_list(authorization_list);
            tx.set_max_fee_per_gas(0);
            tx.set_max_priority_fee_per_gas(0);
        }
        (env, tx)
    }

    fn simulate(
        &mut self,
        transaction: &TransactionMaybeSigned<Ethereum>,
        is_fixed_gas_limit: bool,
    ) -> Result<ScriptResult<Ethereum>> {
        let from = transaction.from().expect("transaction sender missing");
        let calldata = transaction.input().cloned();
        let value = transaction.value().unwrap_or_default();
        let authorization_list = transaction.authorization_list();
        let Some(to) = transaction.to() else {
            let (env, tx) = if is_fixed_gas_limit {
                self.runner.prepare_fixed_gas_env(transaction)
            } else {
                self.runner.executor.prepare_call_env(
                    from,
                    TxKind::Create,
                    calldata.expect("No data for create transaction"),
                    value,
                )
            };
            let result = self
                .transact(env, tx)
                .map_err(EvmError::from)
                .and_then(|raw| self.runner.executor.persist_deployment(raw, None));
            return self.runner.deployment_result(result);
        };

        if is_fixed_gas_limit {
            let (env, tx) = self.runner.prepare_fixed_gas_env(transaction);
            let result = self.transact(env, tx)?;
            let gas_used = result.gas_used;
            return Ok(self.runner.call_result(result, gas_used));
        }

        let calldata = calldata.unwrap_or_default();
        let (env, tx) =
            self.prepare_call(from, to, calldata.clone(), value, authorization_list.clone());
        let result = self.call(env, tx)?;
        let mut gas_used = result.gas_used;
        if result.exit_reason.is_some_and(InstrStop::is_success) {
            let initial_limit = self.runner.executor.tx().gas_limit();
            let mut search = GasSearch::new(gas_used);
            while let Some(limit) = search.next_limit() {
                // Keep the existing gas-probe preparation and authorization behavior unchanged.
                self.runner.executor.tx_mut().set_gas_limit(limit);
                let (env, tx) = self.prepare_call(from, to, calldata.clone(), value, None);
                let result = self.call(env, tx)?;
                search.record(limit, needs_more_gas(result.exit_reason));
            }
            gas_used = search.gas_used();
            match initial_limit {
                Some(gas_limit) => self.runner.executor.tx_mut().set_gas_limit(gas_limit),
                None => self.runner.executor.tx_mut().reset_gas_limit(),
            }
        }

        let (env, tx) = self.prepare_call(from, to, calldata, value, authorization_list);
        let result = self.transact(env, tx)?;
        Ok(self.runner.call_result(result, gas_used))
    }

    fn advance_block(&mut self) {
        if let Some(cursor) = &mut self.cursor {
            cursor.advance_block();
        }
        self.runner.executor.evm_env_mut().block.number += U256::ONE;
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
                    runner: MonadSimulation::new(context.runner).await?,
                    decoder: context.decoder,
                },
            );
        }
        let transactions =
            self.transaction_metadata(|rpc| &contexts.get(rpc).expect("invalid rpc url").decoder)?;
        self.show_simulation_header()?;
        let results = transactions
            .into_iter()
            .map(|transaction| {
                let rpc = transaction.rpc.clone();
                let simulation = &mut contexts.get_mut(&rpc).expect("invalid rpc url").runner;

                let tx = transaction.tx();
                let to = tx.to();
                let result = simulation
                    .simulate(tx, transaction.is_fixed_gas_limit)
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
        let transactions = self.collect_simulation_results(results, &contexts).await?;
        Ok(self.into_filled(transactions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::eip2930::{AccessList, AccessListItem};
    use alloy_primitives::{B256, bytes};
    use evm2::bytecode::Bytecode;
    use foundry_evm::{
        backend::Backend, executors::ExecutorBuilder, hardforks::MonadHardfork, opts::EvmOpts,
    };
    use foundry_evm_networks::NetworkConfigs;

    fn simulation() -> MonadSimulation {
        // Match the simulation environment produced by EvmOpts.
        let opts = EvmOpts { memory_limit: 1 << 25, ..Default::default() };
        let mut env = opts.local_evm_env::<MonadEvmNetwork>();
        env.block.gas_limit = U256::from(30_000_000);
        let executor = ExecutorBuilder::<MonadEvmNetwork>::new().gas_limit(1 << 20).build(
            env,
            Default::default(),
            Backend::spawn(None, env).unwrap(),
            NetworkConfigs::with_monad(),
        );
        let runner = ScriptRunner::new(executor, EvmOpts::default());
        let mut simulation = futures::executor::block_on(MonadSimulation::new(runner)).unwrap();
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
        let result =
            simulation.simulate(&transaction(sender, Some(recipient), None), false).unwrap();
        assert!(result.success);
        assert_transaction_count(&simulation, 1);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 1);

        simulation.advance_block();
        assert_transaction_count(&simulation, 0);
        assert_eq!(simulation.runner.executor.evm_env().block.number, U256::ONE);
        assert_transaction_count(&other, 0);
    }

    #[test]
    fn deployment_and_reverted_call_both_advance_the_cursor() {
        let mut simulation = simulation();
        let sender = Address::with_last_byte(0x42);
        simulation.runner.executor.set_balance(sender, U256::MAX).unwrap();
        // Deploy runtime code that always reverts.
        let initcode = bytes!("6005600c60003960056000f360006000fd");
        let deployment =
            simulation.simulate(&transaction(sender, None, Some(initcode)), false).unwrap();
        assert!(deployment.success);
        assert_transaction_count(&simulation, 1);
        let address = deployment.address.unwrap();
        assert!(!simulation.runner.executor.is_empty_code(address).unwrap());
        let result = simulation.simulate(&transaction(sender, Some(address), None), false).unwrap();
        assert!(!result.success);
        assert_transaction_count(&simulation, 2);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 2);
    }

    #[test]
    fn validation_failure_does_not_advance_the_cursor() {
        let mut simulation = simulation();
        simulation.runner.executor.set_gas_limit(1);
        let sender = Address::with_last_byte(0x42);
        let result = simulation
            .simulate(&transaction(sender, Some(Address::with_last_byte(0x43)), None), false);
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
        assert_eq!(tx.authorization_list(), Some(&Vec::new()));
        assert_transaction_count(&simulation, 0);
    }

    fn transaction(
        from: Address,
        to: Option<Address>,
        input: Option<Bytes>,
    ) -> TransactionMaybeSigned<Ethereum> {
        TransactionMaybeSigned::new(
            TransactionRequest::default()
                .with_from(from)
                .with_kind(to.map_or(TxKind::Create, TxKind::Call))
                .with_input(input.unwrap_or_default()),
        )
    }

    #[test]
    fn fixed_gas_calls_and_deployments_preserve_block_gas_limit() {
        let mut simulation = simulation();
        let sender = Address::with_last_byte(0x42);
        let block_gas_limit = 30_000_000;
        simulation.runner.executor.set_balance(sender, U256::MAX).unwrap();
        simulation.runner.executor.evm_env_mut().block.gas_limit = U256::from(block_gas_limit);
        let executor_gas_limit = simulation.runner.executor.gas_limit();

        // The constructor stores GASLIMIT in slot zero; the runtime returns GASLIMIT.
        let initcode = bytes!("456000556009601060003960096000f34560005260206000f3");
        let deployment = simulation
            .simulate(&fixed_transaction(sender, None, Some(initcode), 100_000), true)
            .unwrap();
        assert!(deployment.success);
        let address = deployment.address.unwrap();
        assert_eq!(
            simulation.runner.executor.backend().storage(address, U256::ZERO).unwrap(),
            U256::from(block_gas_limit),
        );
        assert_transaction_count(&simulation, 1);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 1);

        let result = simulation
            .simulate(&fixed_transaction(sender, Some(address), None, 50_000), true)
            .unwrap();
        assert!(result.success);
        assert_eq!(U256::from_be_slice(&result.returned), U256::from(block_gas_limit));
        assert_transaction_count(&simulation, 2);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 2);
        assert_eq!(simulation.runner.executor.gas_limit(), executor_gas_limit);
        assert_eq!(
            simulation.runner.executor.evm_env().block.gas_limit,
            U256::from(block_gas_limit)
        );
    }

    #[test]
    fn fixed_gas_access_lists_do_not_leak_to_following_transactions() {
        let mut simulation = simulation();
        simulation.runner.executor.set_spec_id(MonadHardfork::MonadTen);
        let sender = Address::with_last_byte(0x42);
        let recipient = Address::with_last_byte(0x43);
        simulation.runner.executor.set_balance(sender, U256::MAX).unwrap();
        let executor_gas_limit = simulation.runner.executor.gas_limit();

        // Read 32 distinct storage pages: warm reads fit the cap, while cold reads do not.
        // MonadTen warms all 128 slots in a page together.
        let mut code = Vec::new();
        for page in 0..32u16 {
            code.push(0x61);
            code.extend_from_slice(&(page * 128).to_be_bytes());
            code.extend_from_slice(&[0x54, 0x50]);
        }
        code.push(0x00);
        simulation.runner.executor.set_code(recipient, Bytecode::new_raw(code.into())).unwrap();
        let access_list = AccessList(vec![AccessListItem {
            address: recipient,
            storage_keys: (0..32).map(|page| B256::from(U256::from(page * 128))).collect(),
        }]);
        let mut transaction = TransactionMaybeSigned::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(recipient)
                .with_gas_price(0)
                .with_gas_limit(100_000)
                .with_access_list(access_list),
        );
        let result = simulation.simulate(&transaction, true).unwrap();
        assert!(result.success);
        assert_transaction_count(&simulation, 1);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 1);

        transaction.as_unsigned_mut().unwrap().access_list = None;
        let result = simulation.simulate(&transaction, true).unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_reason, Some(InstrStop::OutOfGas));
        assert_transaction_count(&simulation, 2);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 2);

        let result = simulation.simulate(&transaction, false).unwrap();
        assert!(result.success);
        assert_transaction_count(&simulation, 3);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 3);
        assert_eq!(simulation.runner.executor.gas_limit(), executor_gas_limit);
    }

    #[test]
    fn fixed_gas_validation_failure_does_not_advance_the_cursor() {
        let mut simulation = simulation();
        let sender = Address::with_last_byte(0x42);
        let recipient = Address::with_last_byte(0x43);
        simulation.runner.executor.set_balance(sender, U256::MAX).unwrap();
        let result =
            simulation.simulate(&fixed_transaction(sender, Some(recipient), None, 1), true);
        assert!(result.is_err());
        assert_transaction_count(&simulation, 0);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 0);

        let result = simulation
            .simulate(&fixed_transaction(sender, Some(recipient), None, 21_000), true)
            .unwrap();
        assert!(result.success);
        assert_transaction_count(&simulation, 1);
        assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), 1);
    }

    fn fixed_transaction(
        from: Address,
        to: Option<Address>,
        input: Option<Bytes>,
        gas_limit: u64,
    ) -> TransactionMaybeSigned<Ethereum> {
        let mut transaction = transaction(from, to, input);
        transaction.as_unsigned_mut().unwrap().set_gas_limit(gas_limit);
        transaction
    }

    #[test]
    fn fixed_gas_failed_deployments_advance_the_cursor() {
        let mut simulation = simulation();
        let sender = Address::with_last_byte(0x42);
        simulation.runner.executor.set_balance(sender, U256::MAX).unwrap();

        for (index, (initcode, exit_reason)) in
            [(bytes!("60006000fd"), InstrStop::Revert), (bytes!("5b600056"), InstrStop::OutOfGas)]
                .into_iter()
                .enumerate()
        {
            let result = simulation
                .simulate(&fixed_transaction(sender, None, Some(initcode), 60_000), true)
                .unwrap();
            assert!(!result.success);
            assert_eq!(result.exit_reason, Some(exit_reason));
            assert_eq!(result.address, Some(Address::ZERO));
            assert_transaction_count(&simulation, index + 1);
            assert_eq!(simulation.runner.executor.get_nonce(sender).unwrap(), index as u64 + 1);
        }
    }
}
