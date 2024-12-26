use std::{any::Any, fmt::Debug};

use alloy_primitives::{Address, U256};
use eyre::Result;
// use foundry_cheatcodes::strategy::{
//     CheatcodeInspectorStrategy, EvmCheatcodeInspectorStrategyRunner,
// };
use foundry_evm_core::backend::{Backend, BackendResult, BackendStrategy, CowBackend};
use revm::{
    primitives::{EnvWithHandlerCfg, ResultAndState},
    DatabaseRef,
};

use crate::inspectors::InspectorStack;

use super::Executor;

pub trait ExecutorStrategyContext: Debug + Send + Sync + Any {
    /// Clone the strategy context.
    fn new_cloned(&self) -> Box<dyn ExecutorStrategyContext>;
    /// Alias as immutable reference of [Any].
    fn as_any_ref(&self) -> &dyn Any;
    /// Alias as mutable reference of [Any].
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl ExecutorStrategyContext for () {
    fn new_cloned(&self) -> Box<dyn ExecutorStrategyContext> {
        Box::new(())
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug)]
pub struct ExecutorStrategy {
    /// Strategy runner.
    pub runner: Box<dyn ExecutorStrategyRunner>,
    /// Strategy context.
    pub context: Box<dyn ExecutorStrategyContext>,
}

impl ExecutorStrategy {
    pub fn new_evm() -> Self {
        Self { runner: Box::new(EvmExecutorStrategyRunner::default()), context: Box::new(()) }
    }
}

impl Clone for ExecutorStrategy {
    fn clone(&self) -> Self {
        Self { runner: self.runner.new_cloned(), context: self.context.new_cloned() }
    }
}

pub trait ExecutorStrategyRunner: Debug + Send + Sync {
    fn name(&self) -> &'static str;

    fn new_cloned(&self) -> Box<dyn ExecutorStrategyRunner>;

    /// Set the balance of an account.
    fn set_balance(
        &self,
        executor: &mut Executor,
        address: Address,
        amount: U256,
    ) -> BackendResult<()>;

    /// Gets the balance of an account
    fn get_balance(&self, executor: &Executor, address: Address) -> BackendResult<U256>;

    /// Set the nonce of an account.
    fn set_nonce(&self, executor: &mut Executor, address: Address, nonce: u64)
        -> BackendResult<()>;

    /// Returns the nonce of an account.
    fn get_nonce(&self, executor: &Executor, address: Address) -> BackendResult<u64>;

    /// Execute a transaction and *WITHOUT* applying state changes.
    fn call(
        &self,
        ctx: &dyn ExecutorStrategyContext,
        backend: &mut CowBackend<'_>,
        env: &mut EnvWithHandlerCfg,
        executor_env: &EnvWithHandlerCfg,
        inspector: &mut InspectorStack,
    ) -> Result<ResultAndState>;

    /// Execute a transaction and apply state changes.
    fn transact(
        &self,
        ctx: &mut dyn ExecutorStrategyContext,
        backend: &mut Backend,
        env: &mut EnvWithHandlerCfg,
        executor_env: &EnvWithHandlerCfg,
        inspector: &mut InspectorStack,
    ) -> Result<ResultAndState>;

    fn new_backend_strategy(&self, ctx: &dyn ExecutorStrategyContext) -> BackendStrategy;

    // fn new_cheatcode_inspector_strategy(
    //     &self,
    //     ctx: &dyn ExecutorStrategyContext,
    // ) -> foundry_cheatcodes::strategy::CheatcodeInspectorStrategy;
}

/// Implements [ExecutorStrategyRunner] for EVM.
#[derive(Debug, Default, Clone)]
pub struct EvmExecutorStrategyRunner {}

impl ExecutorStrategyRunner for EvmExecutorStrategyRunner {
    fn name(&self) -> &'static str {
        "evm"
    }

    fn new_cloned(&self) -> Box<dyn ExecutorStrategyRunner> {
        Box::new(self.clone())
    }

    fn set_balance(
        &self,
        executor: &mut Executor,
        address: Address,
        amount: U256,
    ) -> BackendResult<()> {
        let mut account = executor.backend().basic_ref(address)?.unwrap_or_default();
        account.balance = amount;
        executor.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    fn get_balance(&self, executor: &Executor, address: Address) -> BackendResult<U256> {
        Ok(executor.backend().basic_ref(address)?.map(|acc| acc.balance).unwrap_or_default())
    }

    fn set_nonce(
        &self,
        executor: &mut Executor,
        address: Address,
        nonce: u64,
    ) -> BackendResult<()> {
        let mut account = executor.backend().basic_ref(address)?.unwrap_or_default();
        account.nonce = nonce;
        executor.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    fn get_nonce(&self, executor: &Executor, address: Address) -> BackendResult<u64> {
        Ok(executor.backend().basic_ref(address)?.map(|acc| acc.nonce).unwrap_or_default())
    }

    fn call(
        &self,
        _ctx: &dyn ExecutorStrategyContext,
        backend: &mut CowBackend<'_>,
        env: &mut EnvWithHandlerCfg,
        _executor_env: &EnvWithHandlerCfg,
        inspector: &mut InspectorStack,
    ) -> Result<ResultAndState> {
        backend.inspect(env, inspector, Box::new(()))
    }

    fn transact(
        &self,
        _ctx: &mut dyn ExecutorStrategyContext,
        backend: &mut Backend,
        env: &mut EnvWithHandlerCfg,
        _executor_env: &EnvWithHandlerCfg,
        inspector: &mut InspectorStack,
    ) -> Result<ResultAndState> {
        backend.inspect(env, inspector, Box::new(()))
    }

    fn new_backend_strategy(&self, _ctx: &dyn ExecutorStrategyContext) -> BackendStrategy {
        BackendStrategy::new_evm()
    }

    // fn new_cheatcode_inspector_strategy(
    //     &self,
    //     _ctx: &dyn ExecutorStrategyContext,
    // ) -> CheatcodeInspectorStrategy {
    //     CheatcodeInspectorStrategy {
    //         runner: Box::new(EvmCheatcodeInspectorStrategyRunner::default()),
    //         context: Box::new(()),
    //     }
    // }
}

impl Clone for Box<dyn ExecutorStrategyRunner> {
    fn clone(&self) -> Self {
        self.new_cloned()
    }
}
