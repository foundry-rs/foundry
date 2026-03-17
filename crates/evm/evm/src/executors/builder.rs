use crate::{executors::Executor, inspectors::InspectorStackBuilder};
use foundry_evm_core::{EvmEnv, backend::Backend};
use revm::{context::TxEnv, primitives::hardfork::SpecId};

/// The builder that allows to configure an evm [`Executor`] which a stack of optional
/// [`revm::Inspector`]s, such as [`Cheatcodes`].
///
/// By default, the [`Executor`] will be configured with an empty [`InspectorStack`].
///
/// [`Cheatcodes`]: super::Cheatcodes
/// [`InspectorStack`]: super::InspectorStack
#[derive(Debug, Clone)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct ExecutorBuilder {
    /// The configuration used to build an `InspectorStack`.
    stack: InspectorStackBuilder,
    /// The gas limit.
    gas_limit: Option<u64>,
    /// The spec ID.
    spec_id: SpecId,
    legacy_assertions: bool,
}

impl Default for ExecutorBuilder {
    #[inline]
    fn default() -> Self {
        Self {
            stack: InspectorStackBuilder::new(),
            gas_limit: None,
            spec_id: SpecId::default(),
            legacy_assertions: false,
        }
    }
}

impl ExecutorBuilder {
    /// Create a new executor builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Modify the inspector stack.
    #[inline]
    pub fn inspectors(
        mut self,
        f: impl FnOnce(InspectorStackBuilder) -> InspectorStackBuilder,
    ) -> Self {
        self.stack = f(self.stack);
        self
    }

    /// Sets the EVM spec to use.
    #[inline]
    pub fn spec_id(mut self, spec: SpecId) -> Self {
        self.spec_id = spec;
        self
    }

    /// Sets the executor gas limit.
    #[inline]
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Sets the `legacy_assertions` flag.
    #[inline]
    pub fn legacy_assertions(mut self, legacy_assertions: bool) -> Self {
        self.legacy_assertions = legacy_assertions;
        self
    }

    /// Builds the executor as configured.
    #[inline]
    pub fn build(self, mut evm_env: EvmEnv, tx_env: TxEnv, db: Backend) -> Executor {
        let Self { mut stack, gas_limit, spec_id, legacy_assertions } = self;
        if stack.block.is_none() {
            stack.block = Some(evm_env.block_env.clone());
        }
        if stack.gas_price.is_none() {
            stack.gas_price = Some(tx_env.gas_price);
        }
        let gas_limit = gas_limit.unwrap_or(evm_env.block_env.gas_limit);
        evm_env.cfg_env.set_spec(spec_id);
        Executor::new(db, evm_env, tx_env, stack.build(), gas_limit, legacy_assertions)
    }
}
