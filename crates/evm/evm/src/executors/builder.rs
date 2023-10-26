use crate::{executors::Executor, inspectors::InspectorStackBuilder};
use alloy_primitives::U256;
use foundry_evm_core::backend::Backend;
use revm::primitives::{Env, SpecId};

/// The builder that allows to configure an evm [`Executor`] which a stack of optional
/// [`revm::Inspector`]s, such as [`Cheatcodes`].
///
/// By default, the [`Executor`] will be configured with an empty [`InspectorStack`].
///
/// [`Cheatcodes`]: super::inspector::Cheatcodes
/// [`InspectorStack`]: super::inspector::InspectorStack
#[derive(Debug)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct ExecutorBuilder {
    /// The configuration used to build an [InspectorStack].
    stack: InspectorStackBuilder,
    /// The gas limit.
    gas_limit: Option<U256>,
    /// The spec ID.
    spec_id: SpecId,
}

impl Default for ExecutorBuilder {
    #[inline]
    fn default() -> Self {
        Self { stack: InspectorStackBuilder::new(), gas_limit: None, spec_id: SpecId::LATEST }
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

    /// Sets the EVM spec to use
    #[inline]
    pub fn spec(mut self, spec: SpecId) -> Self {
        self.spec_id = spec;
        self
    }

    /// Sets the executor gas limit.
    ///
    /// See [Executor::gas_limit] for more info on why you might want to set this.
    #[inline]
    pub fn gas_limit(mut self, gas_limit: U256) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Builds the executor as configured.
    #[inline]
    pub fn build(self, mut env: Env, db: Backend) -> Executor {
        let Self { mut stack, gas_limit, spec_id } = self;
        env.cfg.spec_id = spec_id;
        stack.block = Some(env.block.clone());
        stack.gas_price = Some(env.tx.gas_price);
        let gas_limit = gas_limit.unwrap_or(env.block.gas_limit);
        Executor::new(db, env, stack.build(), gas_limit)
    }
}
