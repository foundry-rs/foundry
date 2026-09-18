use alloy_evm::{EvmEnv, EvmFactory, precompiles::PrecompilesMap};
use alloy_op_evm::{OpEvm, OpEvmContext, OpEvmFactory, OpTx};
use foundry_fork_db::DatabaseError;
use op_alloy_network::Optimism;
use op_revm::{
    L1BlockInfo, OpEvm as RevmEvm, OpHaltReason, OpSpecId, OpTransactionError, handler::OpHandler,
};
use revm::{
    context::{
        BlockEnv, Journal,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::{EthFrame, EvmTr, FrameResult, instructions::EthInstructions},
    inspector::InspectorHandler,
    interpreter::{FrameInput, InstructionResult, interpreter::EthInterpreter},
};

use crate::{
    FoundryChain, FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, JournaledState},
    evm::{
        FoundryEvmFactory, FoundryEvmNetwork, IntoInstructionResult, NestedEvm, NestedEvmFor,
        run_inspected_frame,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct OpEvmNetwork;
impl FoundryEvmNetwork for OpEvmNetwork {
    type Network = Optimism;
    type EvmFactory = OpEvmFactory;
}

type OpEvmHandler<'db, I> =
    OpHandler<OpRevmEvm<'db, I>, EVMError<DatabaseError, OpTransactionError>, EthFrame>;

pub type OpRevmEvm<'db, I> = RevmEvm<
    OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>,
    I,
    EthInstructions<EthInterpreter, OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>,
    PrecompilesMap,
>;

impl FoundryChain<OpTx> for L1BlockInfo {}

impl IntoInstructionResult for OpHaltReason {
    fn into_instruction_result(self) -> InstructionResult {
        match self {
            Self::Base(eth) => eth.into(),
            Self::FailedDeposit => InstructionResult::Stop,
        }
    }
}

impl FoundryEvmFactory for OpEvmFactory {
    type Chain = L1BlockInfo;
    type FoundryContext<'db> = OpEvmContext<&'db mut dyn DatabaseExt<Self>>;

    type FoundryEvm<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>> =
        OpEvm<&'db mut dyn DatabaseExt<Self>, I, Self::Precompiles>;

    fn create_foundry_evm_with_inspector<'db, I: FoundryInspectorExt<Self::FoundryContext<'db>>>(
        &self,
        db: &'db mut dyn DatabaseExt<Self>,
        evm_env: EvmEnv<Self::Spec, Self::BlockEnv>,
        inspector: I,
    ) -> Self::FoundryEvm<'db, I> {
        let mut op_evm = Self::default().create_evm_with_inspector(db, evm_env, inspector);
        op_evm.cfg.tx_chain_id_check = true;
        op_evm
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
        Box::new(self.create_foundry_evm_with_inspector(db, evm_env, inspector).into_inner())
    }
}

impl<'db, I: FoundryInspectorExt<OpEvmContext<&'db mut dyn DatabaseExt<OpEvmFactory>>>> NestedEvm
    for OpRevmEvm<'db, I>
{
    type Spec = OpSpecId;
    type Block = BlockEnv;
    type Tx = OpTx;
    type Chain = L1BlockInfo;
    type Journal = Journal<&'db mut dyn DatabaseExt<OpEvmFactory>>;

    fn tx_mut(&mut self) -> &mut Self::Tx {
        self.ctx_mut().tx_mut()
    }

    fn journal_inner_mut(&mut self) -> &mut JournaledState {
        &mut self.ctx().journaled_state.inner
    }

    fn chain_mut(&mut self) -> &mut Self::Chain {
        &mut self.ctx_mut().chain
    }

    fn precompiles_mut(&mut self) -> &mut alloy_evm::precompiles::PrecompilesMap {
        &mut self.0.precompiles
    }

    fn journal_mut(&mut self) -> &mut Self::Journal {
        &mut self.ctx_mut().journaled_state
    }

    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>> {
        run_inspected_frame(self, OpEvmHandler::<I>::new(), frame).map_err(map_op_error)
    }

    fn transact_raw(&mut self, tx: Self::Tx) -> eyre::Result<ResultAndState<HaltReason>> {
        self.ctx().set_tx(tx);

        let mut handler = OpEvmHandler::<I>::new();
        let result = handler.inspect_run(self).map_err(map_op_error)?;

        let result = result.map_haltreason(|h| match h {
            OpHaltReason::Base(eth) => eth,
            _ => HaltReason::PrecompileError,
        });

        Ok(ResultAndState::new(result, self.ctx_ref().journaled_state.inner.state.clone()))
    }

    fn to_evm_env(&self) -> EvmEnv<Self::Spec, Self::Block> {
        self.ctx_ref().evm_clone()
    }
}

/// Maps an OP [`EVMError`] to the common `EVMError<DatabaseError>` used by [`NestedEvm`].
fn map_op_error(e: EVMError<DatabaseError, OpTransactionError>) -> EVMError<DatabaseError> {
    match e {
        EVMError::Database(db) => EVMError::Database(db),
        EVMError::Header(h) => EVMError::Header(h),
        EVMError::Custom(s) => EVMError::Custom(s),
        EVMError::Transaction(t) => EVMError::Custom(format!("op transaction error: {t}")),
        EVMError::CustomAny(custom_any_error) => EVMError::CustomAny(custom_any_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::Backend, evm::EvmEnvFor};
    use alloy_primitives::{Address, TxKind, U256};
    use op_revm::constants::L1_FEE_RECIPIENT;
    use revm::{
        context::{ContextTr, TxEnv},
        inspector::NoOpInspector,
        state::AccountInfo,
    };

    #[test]
    fn constructors_allow_restoring_l1_block_context() {
        let factory = OpEvmFactory::default();
        let mut db = Backend::<OpEvmNetwork>::spawn(None).unwrap();
        let chain = L1BlockInfo {
            l2_block: Some(U256::from(42)),
            l1_base_fee: U256::from(123),
            tx_l1_cost: Some(U256::from(456)),
            ..Default::default()
        };
        {
            let mut evm = factory.create_foundry_evm_with_inspector(
                &mut db,
                EvmEnvFor::<OpEvmNetwork>::default(),
                NoOpInspector,
            );
            *evm.chain_mut() = chain.clone();
            assert_eq!(evm.chain().l2_block, chain.l2_block);
            assert_eq!(evm.chain().l1_base_fee, chain.l1_base_fee);
            assert_eq!(evm.chain().tx_l1_cost, chain.tx_l1_cost);
        }
        let mut evm = factory.create_nested_evm_with_inspector(
            &mut db,
            EvmEnvFor::<OpEvmNetwork>::default(),
            NoOpInspector,
        );
        *evm.chain_mut() = chain.clone();
        assert_eq!(evm.chain_mut().l2_block, chain.l2_block);
        assert_eq!(evm.chain_mut().l1_base_fee, chain.l1_base_fee);
        assert_eq!(evm.chain_mut().tx_l1_cost, chain.tx_l1_cost);
    }

    #[test]
    fn nested_execution_uses_restored_l1_block_context() {
        let caller = Address::with_last_byte(0x42);
        let recipient = Address::with_last_byte(0x43);
        let l1_cost = U256::from(456);
        let mut db = Backend::<OpEvmNetwork>::spawn(None).unwrap();
        db.insert_account_info(caller, AccountInfo { balance: U256::MAX, ..Default::default() });

        let mut evm_env = EvmEnvFor::<OpEvmNetwork>::default();
        evm_env.cfg_env.spec = OpSpecId::REGOLITH;
        evm_env.cfg_env.disable_fee_charge = false;
        let mut evm = OpEvmFactory::default().create_nested_evm(&mut db, evm_env);
        *evm.chain_mut() = L1BlockInfo {
            l2_block: Some(U256::ZERO),
            tx_l1_cost: Some(l1_cost),
            ..Default::default()
        };
        let tx = OpTx(op_revm::OpTransaction {
            base: TxEnv {
                caller,
                gas_limit: 21_000,
                kind: TxKind::Call(recipient),
                ..Default::default()
            },
            enveloped_tx: Some(Default::default()),
            deposit: Default::default(),
        });

        let result = evm.transact_raw(tx).unwrap();

        assert!(result.result.is_success());
        assert_eq!(result.state[&L1_FEE_RECIPIENT].info.balance, l1_cost);
    }
}
