use super::{
    inspector::{Cheatcodes, InspectorStackConfig},
    Executor,
};
use crate::executor::backend::Backend;
use ethers::types::U256;
use foundry_config::cache::StorageCachingConfig;
use revm::{Env, SpecId};

#[derive(Default, Debug)]
pub struct ExecutorBuilder {
    /// The execution environment configuration.
    env: Env,
    /// The configuration used to build an [InspectorStack].
    inspector_config: InspectorStackConfig,
    gas_limit: Option<U256>,
}

// === impl ExecutorBuilder ===

impl ExecutorBuilder {
    /// Enables cheatcodes on the executor.
    #[must_use]
    pub fn with_cheatcodes(mut self, ffi: bool, rpc_storage_caching: StorageCachingConfig) -> Self {
        self.inspector_config.cheatcodes = Some(Cheatcodes::new(
            ffi,
            self.env.block.clone(),
            self.env.tx.gas_price,
            rpc_storage_caching,
        ));
        self
    }

    /// Enables tracing
    #[must_use]
    pub fn with_tracing(self) -> Self {
        self.set_tracing(true)
    }

    /// Sets the tracing verbosity
    #[must_use]
    pub fn set_tracing(mut self, with_tracing: bool) -> Self {
        self.inspector_config.tracing = with_tracing;
        self
    }

    /// Enables the debugger
    #[must_use]
    pub fn with_debugger(mut self) -> Self {
        self.inspector_config.debugger = true;
        self
    }

    /// Sets the EVM spec to use
    #[must_use]
    pub fn with_spec(mut self, spec: SpecId) -> Self {
        self.env.cfg.spec_id = spec;
        self
    }

    /// Sets the executor gas limit.
    ///
    /// See [Executor::gas_limit] for more info on why you might want to set this.
    #[must_use]
    pub fn with_gas_limit(mut self, gas_limit: U256) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Configure the execution environment (gas limit, chain spec, ...)
    #[must_use]
    pub fn with_config(mut self, env: Env) -> Self {
        self.inspector_config.block = env.block.clone();
        self.inspector_config.gas_price = env.tx.gas_price;
        self.env = env;
        self
    }

    /// Builds the executor as configured.
    pub fn build(self, db: Backend) -> Executor {
        let gas_limit = self.gas_limit.unwrap_or(self.env.block.gas_limit);
        Executor::new(db, self.env, self.inspector_config, gas_limit)
    }
}
