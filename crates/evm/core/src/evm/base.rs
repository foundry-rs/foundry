use crate::{
    FoundryChain, FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    constants::SYSTEM_PRECOMPILE_STUB,
    evm::{
        FoundryEvmFactory, FoundryEvmNetwork, IntoInstructionResult, NestedEvm, NestedEvmFor,
        run_inspected_frame,
    },
};
use alloy_evm::{Evm, EvmEnv, EvmFactory, precompiles::PrecompilesMap};
use alloy_primitives::{Address, Bytes};
use base_common_chains::ChainConfig;
use base_common_evm::{
    BaseContext, BaseEvm, BaseEvmFactory, BaseHaltReason, BaseHandler, BaseSpecId, BaseTransaction,
    BaseTransactionError, BaseUpgrade, L1BlockInfo,
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

/// Base precompiles installed at `upgrade` that hold state but carry no bytecode.
///
/// Solidity emits an `extcodesize` check for high-level calls to functions without return data,
/// so a code-less precompile makes the *caller* revert before the precompile ever runs. Base
/// mainnet plants a one-byte sentinel on exactly these accounts, so mirroring it keeps local
/// execution faithful to the chain.
pub fn base_code_sentinel_addresses(upgrade: BaseUpgrade) -> impl Iterator<Item = Address> {
    BASE_CODE_SENTINEL_ADDRESSES
        .iter()
        .copied()
        .filter(move |address| is_base_precompile_active_at(*address, upgrade))
}

impl FoundryChain<BaseTransaction<TxEnv>> for L1BlockInfo {}

impl FoundryEvmFactory for BaseEvmFactory {
    type Chain = L1BlockInfo;
    type FoundryContext<'db> = BaseContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> = BaseRevmEvm<'db, I>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let upgrade = evm_env.cfg_env.spec.upgrade();
        let activation_admin = self.activation_admin_address().or_else(|| {
            ChainConfig::activation_admin_address_for_upgrade_by_chain_id(
                evm_env.cfg_env.chain_id,
                upgrade,
            )
        });
        let factory = self.with_activation_admin_address(activation_admin);
        let mut base_evm = factory.create_evm_with_inspector(db, evm_env, inspector);
        base_evm.ctx_mut().cfg.tx_chain_id_check = true;
        // Preserve existing code, including sentinels already present on forks.
        let sentinel = Bytecode::new_legacy(Bytes::from_static(SYSTEM_PRECOMPILE_STUB));
        let sentinel_hash = sentinel.hash_slow();
        let journal = base_evm.ctx_mut().journal_mut();
        for address in base_code_sentinel_addresses(upgrade) {
            if let Ok(account) = journal.load_account_with_code(address)
                && account.info.code.as_ref().is_none_or(|code| code.is_empty())
            {
                journal.set_code_with_hash(address, sentinel.clone(), sentinel_hash);
            }
        }
        base_evm
    }

    fn create_nested_evm_with_inspector<'db, I>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> NestedEvmFor<'db, Self>
    where
        I: FoundryInspectorExt<Self::FoundryContext<'db>> + 'db,
    {
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector))
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

    fn precompiles_mut(&mut self) -> &mut PrecompilesMap {
        Evm::precompiles_mut(self)
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
    use super::*;
    use crate::backend::Backend;
    use alloy_sol_types::SolCall;
    use base_common_precompiles::{
        ActivationRegistryStorage, B20FactoryStorage, IActivationRegistry, NonceManagerStorage,
        PolicyRegistryStorage, TxContextStorage,
    };
    use revm::{
        ExecuteEvm, context::CfgEnv, inspector::NoOpInspector, primitives::TxKind,
        state::AccountInfo,
    };

    fn base_env(chain_id: u64, upgrade: BaseUpgrade) -> EvmEnv<BaseSpecId, BlockEnv> {
        let mut cfg = CfgEnv::new_with_spec(BaseSpecId::new(upgrade));
        cfg.chain_id = chain_id;
        EvmEnv::new(cfg, BlockEnv::default())
    }

    #[test]
    fn failed_deposit_maps_to_stop() {
        assert_eq!(
            BaseHaltReason::FailedDeposit.into_instruction_result(),
            InstructionResult::Stop
        );
    }

    #[test]
    fn constructor_resolves_activation_admin_and_preserves_override() {
        let chain_admin =
            ChainConfig::activation_admin_address_for_upgrade_by_chain_id(8453, BaseUpgrade::Beryl)
                .unwrap();
        let custom_admin = Address::repeat_byte(0xaa);
        for (override_admin, expected_admin) in
            [(None, chain_admin), (Some(custom_admin), custom_admin)]
        {
            let mut db = Backend::<BaseEvmNetwork>::spawn(None).unwrap();
            let mut evm = BaseEvmFactory::new(override_admin).create_foundry_evm_with_inspector(
                &mut db,
                base_env(8453, BaseUpgrade::Beryl),
                NoOpInspector,
            );
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
                IActivationRegistry::adminCall::abi_decode_returns(result.output().unwrap())
                    .unwrap();
            assert_eq!(admin, expected_admin);
        }
    }

    #[test]
    fn constructor_plants_only_active_registry_sentinels() {
        for upgrade in [BaseUpgrade::Azul, BaseUpgrade::Beryl] {
            let mut db = Backend::<BaseEvmNetwork>::spawn(None).unwrap();
            let mut evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
                &mut db,
                base_env(8453, upgrade),
                NoOpInspector,
            );
            for (address, registry) in [
                (ActivationRegistryStorage::ADDRESS, true),
                (PolicyRegistryStorage::ADDRESS, true),
                (B20FactoryStorage::ADDRESS, false),
                (NonceManagerStorage::ADDRESS, false),
                (TxContextStorage::ADDRESS, false),
            ] {
                let expected = if upgrade == BaseUpgrade::Beryl && registry {
                    Bytecode::new_legacy(Bytes::from_static(SYSTEM_PRECOMPILE_STUB))
                } else {
                    Bytecode::default()
                };
                let account = evm.ctx_mut().journal_mut().load_account_with_code(address).unwrap();
                assert_eq!(
                    account.info.code.as_ref().unwrap(),
                    &expected,
                    "{upgrade:?}: {address}"
                );
                assert_eq!(account.info.code_hash, expected.hash_slow());
            }
        }
    }

    #[test]
    fn constructor_preserves_existing_registry_code() {
        let address = ActivationRegistryStorage::ADDRESS;
        let code = Bytecode::new_legacy(Bytes::from_static(&[0x60, 0x00, 0x00]));
        let code_hash = code.hash_slow();
        let mut db = Backend::<BaseEvmNetwork>::spawn(None).unwrap();
        db.insert_account_info(
            address,
            AccountInfo { code_hash, code: Some(code.clone()), ..Default::default() },
        );
        let mut evm = BaseEvmFactory::default().create_foundry_evm_with_inspector(
            &mut db,
            base_env(8453, BaseUpgrade::Beryl),
            NoOpInspector,
        );
        let account = evm.ctx_mut().journal_mut().load_account_with_code(address).unwrap();
        assert_eq!(account.info.code.as_ref().unwrap(), &code);
        assert_eq!(account.info.code_hash, code_hash);
    }
}
