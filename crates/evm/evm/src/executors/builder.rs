use crate::{executors::Executor, inspectors::InspectorStackBuilder};
use alloy_consensus::transaction::SignerRecoverable;
use alloy_evm::FromRecoveredTx;
use alloy_network::{AnyNetwork, AnyRpcTransaction, Network};
use alloy_rlp::Decodable;
use foundry_evm_core::{EvmEnv, TryAnyToTxEnv, backend::Backend, evm::FoundryEvmFactory};
use foundry_primitives::FoundryTransactionBuilder;
use revm::{
    context::{Block, Transaction},
    primitives::hardfork::SpecId,
};
use std::marker::PhantomData;

/// The builder that allows to configure an evm [`Executor`] which a stack of optional
/// [`revm::Inspector`]s, such as [`Cheatcodes`].
///
/// By default, the [`Executor`] will be configured with an empty [`InspectorStack`].
///
/// [`Cheatcodes`]: super::Cheatcodes
/// [`InspectorStack`]: super::InspectorStack
#[derive(Debug, Clone)]
#[must_use = "builders do nothing unless you call `build` on them"]
pub struct ExecutorBuilder<N: Network, F: FoundryEvmFactory> {
    /// The configuration used to build an `InspectorStack`.
    stack: InspectorStackBuilder<F::BlockEnv>,
    /// The gas limit.
    gas_limit: Option<u64>,
    /// The spec.
    spec: F::Spec,
    legacy_assertions: bool,
    _network: PhantomData<N>,
}

impl<N, F> Default for ExecutorBuilder<N, F>
where
    N: Network<
            TxEnvelope: Decodable + SignerRecoverable,
            TransactionRequest: FoundryTransactionBuilder<N>,
        >,
    F: FoundryEvmFactory<Tx: FromRecoveredTx<N::TxEnvelope>>,
    AnyRpcTransaction: TryAnyToTxEnv<F::Tx>,
{
    #[inline]
    fn default() -> Self {
        Self {
            stack: InspectorStackBuilder::new(),
            gas_limit: None,
            spec: Default::default(),
            legacy_assertions: false,
            _network: PhantomData,
        }
    }
}

impl<N, F> ExecutorBuilder<N, F>
where
    N: Network<
            TxEnvelope: Decodable + SignerRecoverable,
            TransactionRequest: FoundryTransactionBuilder<N>,
        >,
    F: FoundryEvmFactory<Tx: FromRecoveredTx<N::TxEnvelope>, Spec: From<SpecId>>,
    AnyRpcTransaction: TryAnyToTxEnv<F::Tx>,
{
    /// Modify the inspector stack.
    #[inline]
    pub fn inspectors(
        mut self,
        f: impl FnOnce(InspectorStackBuilder<F::BlockEnv>) -> InspectorStackBuilder<F::BlockEnv>,
    ) -> Self {
        self.stack = f(self.stack);
        self
    }

    /// Sets the EVM spec to use.
    #[inline]
    pub fn spec_id(mut self, spec: F::Spec) -> Self {
        self.spec = spec;
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
    pub fn build(
        self,
        mut evm_env: EvmEnv<F::Spec, F::BlockEnv>,
        tx_env: F::Tx,
        db: Backend<AnyNetwork, F>,
    ) -> Executor<N, F> {
        let Self { mut stack, gas_limit, spec, legacy_assertions, .. } = self;
        if stack.block.is_none() {
            stack.block = Some(evm_env.block_env.clone());
        }
        if stack.gas_price.is_none() {
            stack.gas_price = Some(tx_env.gas_price());
        }
        let gas_limit = gas_limit.unwrap_or(evm_env.block_env.gas_limit());
        evm_env.cfg_env.set_spec(spec);
        Executor::new(db, evm_env, tx_env, stack.build(), gas_limit, legacy_assertions)
    }
}
