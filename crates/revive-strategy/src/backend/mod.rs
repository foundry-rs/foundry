use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use foundry_evm::backend::{
    BackendStrategy, BackendStrategyContext, BackendStrategyRunner, EvmBackendStrategyRunner,
    ForkDB,
};
use polkadot_sdk::{sp_core::H160, sp_io};
use revive_env::ExtBuilder;
use serde::{Deserialize, Serialize};

/// Create revive strategy for [BackendStrategy].
pub trait ReviveBackendStrategyBuilder {
    /// Create new revive strategy.
    fn new_revive() -> Self;
}

impl ReviveBackendStrategyBuilder for BackendStrategy {
    fn new_revive() -> Self {
        Self {
            runner: &ReviveBackendStrategyRunner,
            context: Box::new(ReviveBackendStrategyContext::new()),
        }
    }
}

/// revive implementation for [BackendStrategyRunner].
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ReviveBackendStrategyRunner;

impl BackendStrategyRunner for ReviveBackendStrategyRunner {
    fn inspect(
        &self,
        backend: &mut foundry_evm::backend::Backend,
        env: &mut revm::primitives::EnvWithHandlerCfg,
        inspector: &mut dyn foundry_evm::InspectorExt,
        inspect_ctx: Box<dyn std::any::Any>,
    ) -> eyre::Result<revm::primitives::ResultAndState> {
        if !is_revive_inspect_context(inspect_ctx.as_ref()) {
            return EvmBackendStrategyRunner.inspect(backend, env, inspector, inspect_ctx);
        }

        todo!()
    }

    fn update_fork_db(
        &self,
        _ctx: &mut dyn foundry_evm::backend::BackendStrategyContext,
        _active_fork: Option<&foundry_evm::backend::Fork>,
        _mem_db: &foundry_evm::backend::FoundryEvmInMemoryDB,
        _backend_inner: &foundry_evm::backend::BackendInner,
        _active_journaled_state: &mut revm::JournaledState,
        _target_fork: &mut foundry_evm::backend::Fork,
    ) {
        todo!()
    }

    fn merge_journaled_state_data(
        &self,
        _ctx: &mut dyn foundry_evm::backend::BackendStrategyContext,
        _addr: alloy_primitives::Address,
        _active_journaled_state: &revm::JournaledState,
        _fork_journaled_state: &mut revm::JournaledState,
    ) {
        todo!()
    }

    fn merge_db_account_data(
        &self,
        _ctx: &mut dyn foundry_evm::backend::BackendStrategyContext,
        _addr: alloy_primitives::Address,
        _active: &ForkDB,
        _fork_db: &mut ForkDB,
    ) {
        todo!()
    }
}

/// Context for [ReviveBackendStrategyRunner].
#[derive(Debug, Clone)]
pub struct ReviveBackendStrategyContext {
    pub revive_test_externalities: Arc<Mutex<sp_io::TestExternalities>>,
}

impl ReviveBackendStrategyContext {
    fn new() -> Self {
        Self {
            revive_test_externalities: Arc::new(Mutex::new(
                ExtBuilder::default()
                    .balance_genesis_config(vec![(H160::from_low_u64_be(1), 1000)])
                    .build(),
            )),
        }
    }
}

impl BackendStrategyContext for ReviveBackendStrategyContext {
    fn new_cloned(&self) -> Box<dyn BackendStrategyContext> {
        Box::new(self.clone())
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub fn get_backend_ref(ctx: &dyn BackendStrategyContext) -> &ReviveBackendStrategyContext {
    ctx.as_any_ref().downcast_ref().expect("expected ReviveExecutorStrategyContext")
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ReviveInspectContext;

fn is_revive_inspect_context(ctx: &dyn Any) -> bool {
    ctx.downcast_ref::<ReviveInspectContext>().is_some()
}
