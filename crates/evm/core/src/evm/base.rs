use alloy_evm::{Evm, EvmEnv, EvmFactory, precompiles::PrecompilesMap};
use base_common_chains::ChainConfig;
use base_common_evm::{
    BaseContext, BaseEvm, BaseEvmFactory, BaseHaltReason, BaseHandler, BaseSpecId, BaseTransaction,
    BaseTransactionError,
};
use base_common_network::Base;
use foundry_evm_networks::NetworkVariant;
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        BlockEnv, ContextTr, LocalContextTr, TxEnv,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EthFrame, EvmTr, FrameResult, Handler},
    inspector::InspectorHandler,
    interpreter::{
        FrameInput, GasTracker, InstructionResult, SharedMemory, interpreter::EthInterpreter,
        interpreter_action::FrameInit,
    },
};

use crate::{
    FoundryContextExt, FoundryContextState, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{FoundryEvmFactory, FoundryEvmNetwork, IntoInstructionResult, NestedEvm},
};

/// Base EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct BaseEvmNetwork;

impl FoundryEvmNetwork for BaseEvmNetwork {
    type Network = Base;
    type EvmFactory = BaseEvmFactory;

    fn supports_network(network: NetworkVariant) -> bool {
        network.is_base()
    }
}

impl IntoInstructionResult for BaseHaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        match self {
            Self::Base(eth) => eth.into(),
            Self::FailedDeposit => InstructionResult::Stop,
        }
    }
}

pub type BaseRevmEvm<'db, I> = BaseEvm<&'db mut dyn DatabaseExt<BaseEvmFactory>, I, PrecompilesMap>;

type BaseEvmHandler<'db, I> = BaseHandler<
    BaseRevmEvm<'db, I>,
    EVMError<DatabaseError, BaseTransactionError>,
    EthFrame<EthInterpreter>,
>;

fn base_factory_for_env(
    factory: BaseEvmFactory,
    evm_env: &EvmEnv<BaseSpecId, BlockEnv>,
) -> BaseEvmFactory {
    let activation_admin_address = factory.activation_admin_address().or_else(|| {
        ChainConfig::activation_admin_address_for_upgrade_by_chain_id(
            evm_env.cfg_env.chain_id,
            evm_env.cfg_env.spec.upgrade(),
        )
    });
    factory.with_activation_admin_address(activation_admin_address)
}

impl FoundryEvmFactory for BaseEvmFactory {
    type ContextAux = ();
    type FoundryContext<'db> = BaseContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = BaseRevmEvm<'db, I>;

    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        _context_aux: Self::ContextAux,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        let factory = base_factory_for_env(*self, &evm_env);
        factory.create_evm(db, evm_env)
    }

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        _context_aux: Self::ContextAux,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let factory = base_factory_for_env(*self, &evm_env);
        let mut base_evm = factory.create_evm_with_inspector(db, evm_env, inspector);
        base_evm.ctx_mut().cfg.tx_chain_id_check = true;
        base_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        context_aux: Self::ContextAux,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> Box<
        dyn NestedEvm<Spec = BaseSpecId, Block = BlockEnv, Tx = BaseTransaction<TxEnv>, Aux = ()>
            + 'db,
    > {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, context_aux, inspector))
    }
}

fn map_base_error(error: EVMError<DatabaseError, BaseTransactionError>) -> EVMError<DatabaseError> {
    match error {
        EVMError::Database(db) => EVMError::Database(db),
        EVMError::Header(header) => EVMError::Header(header),
        EVMError::Custom(message) => EVMError::Custom(message),
        EVMError::Transaction(transaction) => {
            EVMError::Custom(format!("base transaction error: {transaction}"))
        }
        EVMError::CustomAny(error) => EVMError::CustomAny(error),
    }
}

impl<'db, I: FoundryInspectorExt<BaseContext<&'db mut dyn DatabaseExt<BaseEvmFactory>>>> NestedEvm
    for BaseRevmEvm<'db, I>
{
    type Spec = BaseSpecId;
    type Block = BlockEnv;
    type Tx = BaseTransaction<TxEnv>;
    type Aux = ();

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn context_state(&self) -> FoundryContextState<Self::Aux> {
        self.ctx_ref().context_state()
    }

    fn aux_state(&self) -> Self::Aux {
        self.ctx_ref().aux_state()
    }

    fn set_context_state(&mut self, state: FoundryContextState<Self::Aux>) {
        self.ctx_mut().set_context_state(state);
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        let mut handler = BaseEvmHandler::<I>::new();

        let memory =
            SharedMemory::new_with_buffer(self.ctx_ref().local().shared_memory_buffer().clone());
        let first_frame_input = FrameInit { depth: 0, memory, frame_input: frame };

        let mut frame_result =
            handler.inspect_run_exec_loop(self, first_frame_input).map_err(map_base_error)?;
        let mut parent_gas = GasTracker::new(
            frame_result.gas().limit(),
            frame_result.gas().remaining(),
            frame_result.gas().reservoir(),
        );
        handler
            .last_frame_result(self, &mut frame_result, &mut parent_gas)
            .map_err(map_base_error)?;

        Ok(frame_result)
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>> {
        let ResultAndState { result, state } =
            Evm::transact_raw(self, tx).map_err(map_base_error)?;
        let result = result.map_haltreason(|halt| match halt {
            BaseHaltReason::Base(eth) => eth,
            BaseHaltReason::FailedDeposit => HaltReason::PrecompileError,
        });
        Ok(ResultAndState::new(result, state))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use alloy_sol_types::SolCall;
    use base_common_evm::BaseUpgrade;
    use base_common_precompiles::{
        ActivationRegistryStorage, B20FactoryStorage, IActivationRegistry, NonceManagerStorage,
        TxContextStorage,
    };
    use revm::{
        ExecuteEvm,
        context::CfgEnv,
        database::EmptyDB,
        precompile::secp256r1,
        primitives::{Bytes, TxKind},
    };

    use super::*;
    use crate::evm::{EthEvmNetwork, TempoEvmNetwork};

    fn has_precompile(upgrade: BaseUpgrade, address: Address) -> bool {
        let evm = BaseEvmFactory::default().create_evm(
            EmptyDB::default(),
            EvmEnv::new(CfgEnv::new_with_spec(BaseSpecId::new(upgrade)), BlockEnv::default()),
        );
        evm.precompiles().get(&address).is_some()
    }

    fn base_env(chain_id: u64, upgrade: BaseUpgrade) -> EvmEnv<BaseSpecId, BlockEnv> {
        let mut cfg = CfgEnv::new_with_spec(BaseSpecId::new(upgrade));
        cfg.chain_id = chain_id;
        EvmEnv::new(cfg, BlockEnv::default())
    }

    #[test]
    fn base_evm_factory_implements_foundry_evm_factory() {
        fn assert_foundry_factory<F: FoundryEvmFactory>() {}
        fn assert_foundry_network<N: FoundryEvmNetwork>() {}

        assert_foundry_factory::<BaseEvmFactory>();
        assert_foundry_network::<BaseEvmNetwork>();
        assert!(BaseEvmNetwork::supports_network(NetworkVariant::Base));
        assert!(!EthEvmNetwork::supports_network(NetworkVariant::Base));
        assert!(!TempoEvmNetwork::supports_network(NetworkVariant::Base));
    }

    #[test]
    fn failed_deposit_maps_to_stop() {
        assert_eq!(
            BaseHaltReason::FailedDeposit.into_instruction_result(),
            InstructionResult::Stop
        );
    }

    #[test]
    fn factory_resolves_activation_admin_from_chain_and_upgrade() {
        let env = base_env(8453, BaseUpgrade::Beryl);
        let factory = base_factory_for_env(BaseEvmFactory::default(), &env);
        assert_eq!(
            factory.activation_admin_address(),
            ChainConfig::activation_admin_address_for_upgrade_by_chain_id(8453, BaseUpgrade::Beryl)
        );

        let custom_admin = Address::repeat_byte(0xaa);
        let factory = base_factory_for_env(BaseEvmFactory::new(Some(custom_admin)), &env);
        assert_eq!(factory.activation_admin_address(), Some(custom_admin));
    }

    #[test]
    fn beryl_installs_dynamic_precompiles_after_azul() {
        for address in [B20FactoryStorage::ADDRESS, ActivationRegistryStorage::ADDRESS] {
            assert!(!has_precompile(BaseUpgrade::Azul, address));
            assert!(has_precompile(BaseUpgrade::Beryl, address));
        }
    }

    #[test]
    fn beryl_activation_registry_uses_resolved_chain_admin() {
        let env = base_env(8453, BaseUpgrade::Beryl);
        let factory = base_factory_for_env(BaseEvmFactory::default(), &env);
        let expected_admin = factory.activation_admin_address().unwrap();
        let mut evm = factory.create_evm(EmptyDB::default(), env);
        let tx = BaseTransaction::builder()
            .base(
                TxEnv::builder()
                    .chain_id(Some(8453))
                    .kind(TxKind::Call(ActivationRegistryStorage::ADDRESS))
                    .data(Bytes::from(IActivationRegistry::adminCall {}.abi_encode()))
                    .gas_limit(100_000),
            )
            .build_fill();

        let result = evm.transact_one(tx).unwrap();
        let admin =
            IActivationRegistry::adminCall::abi_decode_returns(result.output().unwrap()).unwrap();
        assert_eq!(admin, expected_admin);
    }

    #[test]
    fn fjord_installs_p256_precompile_after_ecotone() {
        let address = *secp256r1::P256VERIFY.address();
        assert!(!has_precompile(BaseUpgrade::Ecotone, address));
        assert!(has_precompile(BaseUpgrade::Fjord, address));
    }

    #[test]
    fn cobalt_installs_eip8130_precompiles_after_beryl() {
        for address in [TxContextStorage::ADDRESS, NonceManagerStorage::ADDRESS] {
            assert!(!has_precompile(BaseUpgrade::Beryl, address));
            assert!(has_precompile(BaseUpgrade::Cobalt, address));
        }
    }
}
