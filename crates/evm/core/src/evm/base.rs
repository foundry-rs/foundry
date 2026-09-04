use alloy_evm::{Evm, EvmEnv, EvmFactory, precompiles::PrecompilesMap};
use alloy_primitives::{Address, Bytes};
use base_common_chains::ChainConfig;
use base_common_evm::{
    BaseContext, BaseEvm, BaseEvmFactory, BaseHaltReason, BaseHandler, BaseSpecId, BaseTransaction,
    BaseTransactionError, L1BlockInfo,
};
use base_common_network::Base;
// Only the tests below need this crate, but Cargo forbids optional dev-dependencies, so it is an
// optional regular dependency that the `base` feature turns on.
use base_common_precompiles as _;
use foundry_evm_networks::{BASE_CODE_SENTINEL_ADDRESSES, is_base_precompile_active_at};
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        BlockEnv, ContextTr, Journal, JournalTr, TxEnv,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EthFrame, EvmTr, FrameResult},
    interpreter::{FrameInput, InstructionResult, interpreter::EthInterpreter},
    state::Bytecode,
};

use crate::{
    FoundryChain, FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    constants::SYSTEM_PRECOMPILE_STUB,
    evm::{
        FoundryEvmFactory, FoundryEvmNetwork, IntoInstructionResult, NestedEvm, NestedEvmFor,
        run_inspected_frame,
    },
};

/// Base EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct BaseEvmNetwork;

impl FoundryEvmNetwork for BaseEvmNetwork {
    type Network = Base;
    type EvmFactory = BaseEvmFactory;
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

/// Base precompiles installed at `spec` that hold state but carry no bytecode.
///
/// Solidity emits an `extcodesize` check for high-level calls to functions without return data,
/// so a code-less precompile makes the *caller* revert before the precompile ever runs. Base
/// mainnet plants a one-byte sentinel on exactly these accounts, so mirroring it keeps local
/// execution faithful to the chain.
pub fn base_code_sentinel_addresses(spec: BaseSpecId) -> impl Iterator<Item = Address> {
    let upgrade = spec.upgrade();
    BASE_CODE_SENTINEL_ADDRESSES
        .iter()
        .copied()
        .filter(move |address| is_base_precompile_active_at(*address, upgrade))
}

/// Plants the sentinel byte on code-less stateful precompiles for a newly created EVM.
///
/// A real deployment is never replaced, so forks that already carry the mainnet sentinel — or
/// any genuine code — are left untouched.
fn plant_code_sentinels<'db, I>(evm: &mut BaseRevmEvm<'db, I>)
where
    I: FoundryInspectorExt<BaseContext<&'db mut dyn DatabaseExt<BaseEvmFactory>>>,
{
    let spec = evm.ctx_ref().cfg_env().spec;
    let sentinel = Bytecode::new_legacy(Bytes::from_static(SYSTEM_PRECOMPILE_STUB));
    let sentinel_hash = sentinel.hash_slow();
    let journal = evm.ctx_mut().journal_mut();
    for address in base_code_sentinel_addresses(spec) {
        let Ok(account) = journal.load_account_with_code(address) else { continue };
        let is_code_less = account.info.code.as_ref().is_none_or(|code| code.is_empty());
        if is_code_less {
            journal.set_code_with_hash(address, sentinel.clone(), sentinel_hash);
        }
    }
}

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

impl FoundryChain<BaseTransaction<TxEnv>> for L1BlockInfo {}

impl FoundryEvmFactory for BaseEvmFactory {
    type Chain = L1BlockInfo;
    type FoundryContext<'db> = BaseContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = BaseRevmEvm<'db, I>;

    fn create_evm_with_context<DB: alloy_evm::Database>(
        &self,
        db: DB,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
    ) -> Self::Evm<DB, revm::inspector::NoOpInspector> {
        let factory = base_factory_for_env(*self, &evm_env);
        let mut evm = factory.create_evm(db, evm_env);
        evm.ctx_mut().chain = chain_context;
        evm
    }

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let factory = base_factory_for_env(*self, &evm_env);
        let mut base_evm = factory.create_evm_with_inspector(db, evm_env, inspector);
        base_evm.ctx_mut().chain = chain_context;
        base_evm.ctx_mut().cfg.tx_chain_id_check = true;
        plant_code_sentinels(&mut base_evm);
        base_evm
    }

    fn create_foundry_nested_evm<'db>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        chain_context: Self::Chain,
        inspector: &'db mut dyn FoundryInspectorExt<Self::FoundryContext<'db>>,
    ) -> NestedEvmFor<'db, Self> {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, chain_context, inspector))
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
    type Chain = L1BlockInfo;
    type Journal = Journal<&'db mut dyn DatabaseExt<BaseEvmFactory>>;

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx_mut().journaled_state.inner
    }

    fn tx_mut(&mut self) -> &mut Self::Tx {
        self.ctx_mut().tx_mut()
    }

    fn chain_mut(&mut self) -> &mut Self::Chain {
        &mut self.ctx_mut().chain
    }

    fn journal_mut(&mut self) -> &mut Self::Journal {
        &mut self.ctx_mut().journaled_state
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        run_inspected_frame(self, BaseEvmHandler::<I>::new(), frame).map_err(map_base_error)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState<HaltReason>> {
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
    use alloy_sol_types::SolCall;
    use base_common_evm::BaseUpgrade;
    use base_common_precompiles::{
        ActivationRegistryStorage, B20FactoryStorage, IActivationRegistry, NonceManagerStorage,
        PolicyRegistryStorage, TxContextStorage,
    };
    use revm::{
        ExecuteEvm, context::CfgEnv, database::EmptyDB, precompile::secp256r1, primitives::TxKind,
    };

    use super::*;

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

    /// Mainnet plants a one-byte sentinel on exactly the two registries that expose
    /// void-returning functions, and leaves the factory, nonce manager, and transaction context
    /// code-less. Stubbing more than this would make an `isContract` probe pass locally and
    /// revert on Base.
    #[test]
    fn code_sentinel_covers_only_void_returning_precompiles() {
        let stubbed =
            |upgrade| base_code_sentinel_addresses(BaseSpecId::new(upgrade)).collect::<Vec<_>>();

        assert!(stubbed(BaseUpgrade::Azul).is_empty());

        for upgrade in [BaseUpgrade::Beryl, BaseUpgrade::Cobalt] {
            let addresses = stubbed(upgrade);
            assert_eq!(
                addresses,
                vec![ActivationRegistryStorage::ADDRESS, PolicyRegistryStorage::ADDRESS],
                "unexpected sentinel set at {upgrade:?}"
            );
            for absent in [
                B20FactoryStorage::ADDRESS,
                NonceManagerStorage::ADDRESS,
                TxContextStorage::ADDRESS,
            ] {
                assert!(
                    !addresses.contains(&absent),
                    "{absent} is code-less on Base and must not be stubbed"
                );
            }
        }
    }

    #[test]
    fn base_evm_factory_implements_foundry_evm_factory() {
        fn assert_foundry_factory<F: FoundryEvmFactory>() {}
        fn assert_foundry_network<N: FoundryEvmNetwork>() {}

        assert_foundry_factory::<BaseEvmFactory>();
        assert_foundry_network::<BaseEvmNetwork>();
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
