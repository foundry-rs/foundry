//! Mutable execution context access.

use super::{EvmEnv, FoundryBlock, FoundryCfg, FoundryChain, FoundryJournal, FoundryTransaction};
use crate::backend::JournaledState;
use revm::{
    Context, Database,
    context::{Cfg, CfgEnv},
    context_interface::ContextTr,
    primitives::hardfork::SpecId,
};
use std::fmt::Debug;

/// Extension trait providing mutable field access to block, tx, and cfg environments.
///
/// [`ContextTr`] only exposes immutable references for block, tx, and cfg.
/// Cheatcodes like `vm.warp()`, `vm.roll()`, `vm.chainId()` need to mutate these fields.
pub trait FoundryContextExt:
    ContextTr<
        Block: FoundryBlock + Clone,
        Tx: FoundryTransaction + Clone,
        Cfg: FoundryCfg<Spec = Self::Spec>,
        Journal: FoundryJournal,
        Chain: FoundryChain<Self::Tx>,
    >
{
    /// Specification id type
    ///
    /// Bubbled-up from `ContextTr::Cfg` for convenience and simplified bounds.
    type Spec: Into<SpecId> + Copy + Debug;

    /// Mutable reference to the block environment.
    fn block_mut(&mut self) -> &mut Self::Block;

    /// Mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut Self::Tx;

    /// Mutable reference to the configuration environment.
    fn cfg_mut(&mut self) -> &mut Self::Cfg;

    /// Reference to the underlying [`CfgEnv`].
    fn cfg_env(&self) -> &CfgEnv<Self::Spec> {
        self.cfg().cfg_env()
    }

    /// Mutable reference to the underlying [`CfgEnv`].
    fn cfg_env_mut(&mut self) -> &mut CfgEnv<Self::Spec> {
        self.cfg_mut().cfg_env_mut()
    }

    /// Mutable reference to the db and the journal inner.
    fn db_journal_inner_mut(&mut self) -> (&mut Self::Db, &mut JournaledState) {
        self.journal_mut().db_journal_inner_mut()
    }

    /// Reference to the journal inner.
    fn journal_inner(&self) -> &JournaledState {
        self.journal().journal_inner()
    }

    /// Sets the spec and refreshes gas params for the concrete EVM family.
    fn set_spec_and_gas_params(&mut self, spec: Self::Spec) {
        self.cfg_mut().set_spec_and_gas_params(spec);
    }

    /// Sets block environment.
    fn set_block(&mut self, block: Self::Block) {
        *self.block_mut() = block;
    }

    /// Sets transaction environment.
    fn set_tx(&mut self, tx: Self::Tx) {
        *self.tx_mut() = tx;
    }

    /// Sets configuration environment.
    fn set_cfg(&mut self, cfg: Self::Cfg) {
        *self.cfg_mut() = cfg;
    }

    /// Sets journal inner.
    fn set_journal_inner(&mut self, journal_inner: JournaledState) {
        *self.db_journal_inner_mut().1 = journal_inner;
    }

    /// Sets EVM environment.
    fn set_evm(&mut self, evm_env: EvmEnv<Self::Spec, Self::Block>) {
        *self.cfg_mut() = evm_env.cfg_env.into();
        *self.block_mut() = evm_env.block_env;
    }

    /// Cloned transaction environment.
    fn tx_clone(&self) -> Self::Tx {
        self.tx().clone()
    }

    /// Cloned EVM environment (Cfg + Block).
    fn evm_clone(&self) -> EvmEnv<Self::Spec, Self::Block> {
        EvmEnv::new(self.cfg().clone().into(), self.block().clone())
    }
}

impl<
    BLOCK: FoundryBlock + Clone,
    TX: FoundryTransaction + Clone,
    CFG: FoundryCfg,
    DB: Database,
    J: FoundryJournal<Database = DB>,
    C: FoundryChain<TX>,
> FoundryContextExt for Context<BLOCK, TX, CFG, DB, J, C>
{
    type Spec = <Self::Cfg as Cfg>::Spec;

    fn block_mut(&mut self) -> &mut Self::Block {
        &mut self.block
    }

    fn tx_mut(&mut self) -> &mut Self::Tx {
        &mut self.tx
    }

    fn cfg_mut(&mut self) -> &mut Self::Cfg {
        &mut self.cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_evm::{EthEvmFactory, EvmFactory};
    use alloy_primitives::U256;
    use foundry_evm_hardforks::TempoHardfork;
    use revm::{
        context::{Block, Transaction},
        database::EmptyDB,
    };
    use tempo_evm::TempoEvmFactory;

    #[cfg(feature = "base")]
    use base_common_evm::{BaseEvmFactory, BaseSpecId, BaseUpgrade};

    #[test]
    fn eth_evm_foundry_context_ext_implementation() {
        let mut evm = EthEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());
        assert_context_mutation(evm.ctx_mut(), SpecId::AMSTERDAM);
    }

    #[cfg(feature = "base")]
    #[test]
    fn base_evm_foundry_context_ext_implementation() {
        let mut evm = BaseEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());
        assert_context_mutation(evm.ctx_mut(), BaseSpecId::new(BaseUpgrade::Beryl));
    }

    #[cfg(feature = "monad")]
    #[test]
    fn monad_evm_foundry_context_ext_implementation() {
        let mut evm = alloy_monad_evm::MonadEvmFactory::default().create_evm(
            EmptyDB::default(),
            EvmEnv::new(
                CfgEnv::new_with_spec(monad_revm::MonadHardfork::MonadNine),
                Default::default(),
            ),
        );
        assert_context_mutation(evm.ctx_mut(), monad_revm::MonadHardfork::MonadEight);
        evm.ctx_mut().journal_mut().set_preserve_reserve_balance(true);
        let mut inner = evm.ctx().journal_inner().clone();
        inner.depth = 2;
        evm.ctx_mut().set_journal_inner(inner);
        assert_eq!(evm.ctx().journal_inner().depth, 2);
        assert!(evm.ctx().journal().preserves_reserve_balance());
    }

    #[test]
    fn tempo_evm_foundry_context_ext_implementation() {
        let mut evm = TempoEvmFactory::default().create_evm(EmptyDB::default(), EvmEnv::default());
        assert_context_mutation(evm.ctx_mut(), TempoHardfork::Genesis);
    }

    #[cfg(feature = "optimism")]
    mod optimism {
        use super::*;
        use alloy_op_evm::{OpEvmFactory, OpTx};
        use op_revm::OpSpecId;

        #[test]
        fn op_evm_foundry_context_ext_implementation() {
            let mut evm =
                OpEvmFactory::<OpTx>::default().create_evm(EmptyDB::default(), EvmEnv::default());
            assert_context_mutation(evm.ctx_mut(), OpSpecId::JOVIAN);
        }
    }

    fn assert_context_mutation<CTX: FoundryContextExt>(ctx: &mut CTX, spec: CTX::Spec)
    where
        CTX::Spec: PartialEq,
    {
        ctx.block_mut().set_number(U256::from(123));
        assert_eq!(ctx.block().number(), U256::from(123));
        ctx.tx_mut().set_nonce(99);
        assert_eq!(ctx.tx().nonce(), 99);
        ctx.cfg_env_mut().spec = spec;
        assert_eq!(ctx.cfg().spec(), spec);

        let tx = ctx.tx_clone();
        ctx.tx_mut().set_nonce(0);
        ctx.set_tx(tx);
        assert_eq!(ctx.tx().nonce(), 99);
        ctx.cfg_env_mut().chain_id = 42;
        let env = ctx.evm_clone();
        ctx.block_mut().set_number(U256::ZERO);
        ctx.cfg_env_mut().chain_id = 43;
        ctx.set_evm(env);
        assert_eq!(ctx.block().number(), U256::from(123));
        assert_eq!(ctx.cfg().spec(), spec);
        assert_eq!(ctx.cfg().chain_id(), 42);
    }
}
