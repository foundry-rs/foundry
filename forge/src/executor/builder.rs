use revm::{db::EmptyDB, Env, SpecId};

use super::Executor;

pub struct ExecutorBuilder {
    /// Whether or not cheatcodes are enabled
    cheatcodes: bool,
    /// Whether or not the FFI cheatcode is enabled
    ffi: bool,
    /// The execution environment configuration.
    config: Env,
}

impl ExecutorBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self { cheatcodes: false, ffi: false, config: Env::default() }
    }

    /// Enables cheatcodes on the executor.
    #[must_use]
    pub fn with_cheatcodes(mut self, ffi: bool) -> Self {
        self.cheatcodes = true;
        self.ffi = ffi;
        self
    }

    pub fn with_spec(mut self, spec: SpecId) -> Self {
        self.config.cfg.spec_id = spec;
        self
    }

    /// Configure the execution environment (gas limit, chain spec, ...)
    #[must_use]
    pub fn with_config(mut self, config: Env) -> Self {
        self.config = config;
        self
    }

    /// Builds the executor as configured.
    pub fn build(self) -> Executor<EmptyDB> {
        Executor::new(EmptyDB(), self.config)
    }

    // TODO: add with_traces
    // TODO: add with_debug(ger?)
    // TODO: add forked
}
