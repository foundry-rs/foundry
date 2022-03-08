use revm::{db::EmptyDB, Env, SpecId};

use super::{inspector::InspectorStackConfig, Executor};

#[derive(Default)]
pub struct ExecutorBuilder {
    /// The execution environment configuration.
    env: Env,
    /// The configuration used to build an [InspectorStack].
    inspector_config: InspectorStackConfig,
}

impl ExecutorBuilder {
    #[must_use]
    pub fn new() -> Self {
        Default::default()
    }

    /// Enables cheatcodes on the executor.
    #[must_use]
    pub fn with_cheatcodes(mut self, ffi: bool) -> Self {
        self.inspector_config.cheatcodes = true;
        self.inspector_config.ffi = ffi;
        self
    }

    pub fn with_spec(mut self, spec: SpecId) -> Self {
        self.env.cfg.spec_id = spec;
        self
    }

    /// Configure the execution environment (gas limit, chain spec, ...)
    #[must_use]
    pub fn with_config(mut self, env: Env) -> Self {
        self.env = env;
        self
    }

    /// Builds the executor as configured.
    pub fn build(self) -> Executor<EmptyDB> {
        Executor::new(EmptyDB(), self.env, self.inspector_config)
    }

    // TODO: add with_traces
    // TODO: add with_debug(ger?)
    // TODO: add forked
}
