use crate::{
    executors::Executor,
    inspectors::{InspectorStackBuilder, TempoLabels},
};
use alloy_primitives::Address;
#[cfg(feature = "base")]
use foundry_evm_core::evm::BaseEvmNetwork;
#[cfg(feature = "optimism")]
use foundry_evm_core::evm::OpEvmNetwork;
use foundry_evm_core::{
    backend::Backend,
    evm::{
        BlockEnvFor, EthEvmNetwork, EvmEnvFor, FoundryEvmNetwork, SpecFor, TempoEvmNetwork,
        TxEnvFor,
    },
};
#[cfg(feature = "monad")]
use foundry_evm_core::{constants::MONAD_CHEATCODE_ADDRESS, evm::MonadEvmNetwork};
use foundry_evm_networks::NetworkConfigs;
use revm::context::{Block, Transaction};

/// The builder that allows to configure an evm [`Executor`] which a stack of optional
/// [`revm::Inspector`]s, such as [`Cheatcodes`].
///
/// By default, the [`Executor`] will be configured with an empty [`InspectorStack`] and no
/// network-specific tooling. Command dispatch should use the concrete FEN's inherent `new`
/// constructor so any required tooling is selected there.
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
            stack: InspectorStackBuilder::new().extra_cheatcode_addresses(&[]),
            gas_limit: None,
            spec: None,
            legacy_assertions: false,
        }
    }
}

impl<FEN: FoundryEvmNetwork> ExecutorBuilder<FEN> {
    /// Returns additional cheatcode addresses selected for this executor.
    #[inline]
    pub const fn extra_cheatcode_addresses(&self) -> &'static [Address] {
        self.stack.extra_cheatcode_addresses
    }

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
        // TODO(monad-fen-dispatch): Remove this argument after inspector inputs and backend fork
        // behavior are resolved by the initial concrete FEN dispatch.
        networks: NetworkConfigs,
    ) -> Executor<FEN> {
        let Self { mut stack, gas_limit, spec, legacy_assertions, .. } = self;
        stack.networks = networks;
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
        Executor::new(db, evm_env, tx_env, stack.build(), networks, gas_limit, legacy_assertions)
    }
}

impl ExecutorBuilder<EthEvmNetwork> {
    /// Creates the default Ethereum executor builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "base")]
impl ExecutorBuilder<BaseEvmNetwork> {
    /// Creates the default Base executor builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "optimism")]
impl ExecutorBuilder<OpEvmNetwork> {
    /// Creates the default OP executor builder.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExecutorBuilder<TempoEvmNetwork> {
    /// Creates a Tempo executor builder with its native label inspector.
    #[inline]
    pub fn new() -> Self {
        Self::default().inspectors(|stack| stack.tempo_labels(TempoLabels::default()))
    }
}

#[cfg(feature = "monad")]
impl ExecutorBuilder<MonadEvmNetwork> {
    /// Creates a Monad executor builder with MonadVM cheatcode support.
    #[inline]
    pub fn new() -> Self {
        Self::default()
            .inspectors(|stack| stack.extra_cheatcode_addresses(&[MONAD_CHEATCODE_ADDRESS]))
    }
}
