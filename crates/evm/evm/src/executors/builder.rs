use crate::{
    executors::Executor,
    inspectors::{InspectorStackBuilder, TempoLabels},
};
use alloy_network::TransactionBuilder;
use alloy_primitives::Address;
use foundry_common::FoundryTransactionBuilder;
use foundry_evm_core::{
    backend::Backend,
    evm::{
        BlockEnvFor, EthEvmNetwork, EvmEnvFor, EvmFactoryFor, FoundryEvmFactory, FoundryEvmNetwork,
        SpecFor, TempoEvmNetwork, TransactionRequestFor,
    },
};
use foundry_evm_networks::NetworkConfigs;

#[cfg(feature = "base")]
use foundry_evm_core::evm::BaseEvmNetwork;

#[cfg(feature = "monad")]
use foundry_evm_core::{constants::MONAD_CHEATCODE_ADDRESS, evm::MonadEvmNetwork};

#[cfg(feature = "optimism")]
use foundry_evm_core::evm::OpEvmNetwork;

/// The builder that allows to configure an evm [`Executor`] which a stack of optional
/// evm2 inspectors, such as [`Cheatcodes`].
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
    /// The spec override. When `None`, the spec of the environment is preserved.
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

    /// Optionally sets the EVM spec. When `None`, the spec of the environment is preserved.
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
    ///
    /// `tx` holds the transaction defaults, such as the sender and gas price, of the executor's
    /// calls.
    #[inline]
    pub fn build(
        self,
        mut evm_env: EvmEnvFor<FEN>,
        tx: TransactionRequestFor<FEN>,
        db: Backend<FEN>,
        // TODO(monad-fen-lifecycle): Remove this argument after the backend's Monad fork-position
        // migration and the Celo factory configuration cleanup are complete.
        networks: NetworkConfigs,
    ) -> Executor<FEN> {
        let Self { mut stack, gas_limit, spec, legacy_assertions, .. } = self;
        if stack.block.is_none() {
            stack.block = Some(evm_env.block);
        }
        if stack.gas_price.is_none() {
            stack.gas_price =
                tx.gas_price().or_else(|| tx.max_fee_per_gas()).filter(|price| *price != 0);
        }
        let gas_limit = gas_limit.unwrap_or_else(|| evm_env.block.gas_limit.saturating_to());
        if let Some(spec) = spec {
            evm_env = EvmFactoryFor::<FEN>::default().env_with_spec(evm_env, spec);
        }
        let mut inspector = stack.build();
        if let Some(cheats) = inspector.cheatcodes.as_mut() {
            // Broadcast requests record the fee token of the executor's transactions.
            cheats.fee_token = tx.fee_token();
        }
        Executor::new(db, evm_env, tx, inspector, networks, gas_limit, legacy_assertions)
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
