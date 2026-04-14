use crate::{executors::Executor, inspectors::InspectorStackBuilder};
use foundry_evm_core::{
    backend::Backend,
    evm::{BlockEnvFor, EvmEnvFor, FoundryEvmNetwork, SpecFor, TxEnvFor},
};
use revm::context::{Block, Transaction};

/// The builder that allows to configure an evm [`Executor`] which a stack of optional
/// [`revm::Inspector`]s, such as [`Cheatcodes`].
///
/// By default, the [`Executor`] will be configured with an empty [`InspectorStack`].
///
/// [`Cheatcodes`]: super::Cheatcodes
/// [`InspectorStack`]: super::InspectorStack
#[derive(Debug, Clone)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct ExecutorBuilder<FEN: FoundryEvmNetwork> {
    /// The configuration used to build an `InspectorStack`.
    stack: InspectorStackBuilder<BlockEnvFor<FEN>>,
    /// The gas limit.
    gas_limit: Option<u64>,
    /// The spec override. When `None`, the spec from `EvmEnv::cfg_env` is preserved.
    spec: Option<SpecFor<FEN>>,
    legacy_assertions: bool,
}

impl<FEN: FoundryEvmNetwork> Default for ExecutorBuilder<FEN> {
    #[inline]
    fn default() -> Self {
        Self {
            stack: InspectorStackBuilder::new(),
            gas_limit: None,
            spec: None,
            legacy_assertions: false,
        }
    }
}

impl<FEN: FoundryEvmNetwork> ExecutorBuilder<FEN> {
    /// Modify the inspector stack.
    #[inline]
    pub fn inspectors(
        mut self,
        f: impl FnOnce(
            InspectorStackBuilder<BlockEnvFor<FEN>>,
        ) -> InspectorStackBuilder<BlockEnvFor<FEN>>,
    ) -> Self {
        self.stack = f(self.stack);
        self
    }

    /// Sets the EVM spec to use.
    #[inline]
    pub const fn spec_id(mut self, spec: SpecFor<FEN>) -> Self {
        self.spec = Some(spec);
        self
    }

    /// Optionally sets the EVM spec. When `None`, the spec from `EvmEnv::cfg_env` is preserved.
    #[inline]
    pub const fn spec_id_opt(self, spec: Option<SpecFor<FEN>>) -> Self {
        if let Some(spec) = spec { self.spec_id(spec) } else { self }
    }

    /// Sets the executor gas limit.
    #[inline]
    pub const fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Sets the `legacy_assertions` flag.
    #[inline]
    pub const fn legacy_assertions(mut self, legacy_assertions: bool) -> Self {
        self.legacy_assertions = legacy_assertions;
        self
    }

    /// Builds the executor as configured.
    #[inline]
    pub fn build(
        self,
        mut evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
        db: Backend<FEN>,
    ) -> Executor<FEN> {
        let Self { mut stack, gas_limit, spec, legacy_assertions, .. } = self;
        if stack.block.is_none() {
            stack.block = Some(evm_env.block_env.clone());
        }
        if stack.gas_price.is_none() {
            stack.gas_price = Some(tx_env.gas_price());
        }
        let gas_limit = gas_limit.unwrap_or(evm_env.block_env.gas_limit());
        if let Some(spec) = spec {
            evm_env.cfg_env.set_spec_and_mainnet_gas_params(spec);
        }
        Executor::new(db, evm_env, tx_env, stack.build(), gas_limit, legacy_assertions)
    }
}
