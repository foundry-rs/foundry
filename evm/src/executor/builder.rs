use super::{
    inspector::{Cheatcodes, Fuzzer, InspectorStackConfig, OnLog},
    Executor,
};
use crate::{
    executor::{backend::Backend, inspector::CheatsConfig},
    fuzz::{invariant::RandomCallGenerator, strategies::EvmFuzzState},
};
use ethers::types::U256;
use revm::primitives::{Env, SpecId};

/// The builder that allows to configure an evm [`Executor`] which a stack of optional
/// [`revm::Inspector`]s, such as [`Cheatcodes`]
///
/// By default, the [`Executor`] will be configured with an empty [`InspectorStack`]
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
    pub fn with_cheatcodes(mut self, config: CheatsConfig) -> Self {
        self.inspector_config.cheatcodes =
            Some(Cheatcodes::new(self.env.block.clone(), self.env.tx.gas_price.into(), config));
        self
    }

    /// Enables or disables tracing
    #[must_use]
    pub fn set_tracing(mut self, enable: bool) -> Self {
        self.inspector_config.tracing = enable;
        self
    }

    /// Enables or disables the debugger
    #[must_use]
    pub fn set_debugger(mut self, enable: bool) -> Self {
        self.inspector_config.debugger = enable;
        self
    }

    /// Enables or disables coverage collection
    #[must_use]
    pub fn set_coverage(mut self, enable: bool) -> Self {
        self.inspector_config.coverage = enable;
        self
    }

    /// Enables or disabled trace printer.
    #[must_use]
    pub fn set_trace_printer(mut self, enable: bool) -> Self {
        self.inspector_config.trace_printer = enable;
        self
    }

    /// Enables the fuzzer for data collection and maybe call overriding
    #[must_use]
    pub fn with_fuzzer(
        mut self,
        call_generator: Option<RandomCallGenerator>,
        fuzz_state: EvmFuzzState,
    ) -> Self {
        self.inspector_config.fuzzer = Some(Fuzzer { call_generator, fuzz_state, collect: false });
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
        self.inspector_config.gas_price = env.tx.gas_price.into();
        self.env = env;
        self
    }

    /// Enable the chisel state inspector
    #[must_use]
    pub fn with_chisel_state(mut self, final_pc: usize) -> Self {
        self.inspector_config.chisel_state = Some(final_pc);
        self
    }

    /// Builds the executor as configured.
    pub fn build<ONLOG: OnLog>(self, db: Backend) -> Executor<ONLOG> {
        let gas_limit = self.gas_limit.unwrap_or(self.env.block.gas_limit.into());
        Executor::new(db, self.env, self.inspector_config, gas_limit)
    }
}
