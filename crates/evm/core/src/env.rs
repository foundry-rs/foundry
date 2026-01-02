pub use alloy_evm::EvmEnv;
use monad_revm::{MonadCfgEnv, MonadSpecId};
use revm::{
    Context, Database, Journal, JournalEntry,
    context::{BlockEnv, CfgEnv, JournalInner, JournalTr, TxEnv},
    primitives::hardfork::SpecId,
};

/// Type alias for Monad-specific CfgEnv.
pub type MonadCfg = CfgEnv<MonadSpecId>;

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
///
/// Uses [`MonadSpecId`] for Monad-specific hardfork identification.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv<MonadSpecId>,
    pub tx: TxEnv,
}

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
impl Env {
    pub fn default_with_spec_id(spec_id: MonadSpecId) -> Self {
        let mut cfg = MonadCfg::default();
        cfg.spec = spec_id;

        Self::from(cfg, BlockEnv::default(), TxEnv::default())
    }

    pub fn from(cfg: MonadCfg, block: BlockEnv, tx: TxEnv) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx }
    }

    pub fn new_with_spec_id(cfg: MonadCfg, block: BlockEnv, tx: TxEnv, spec_id: MonadSpecId) -> Self {
        let mut cfg = cfg;
        cfg.spec = spec_id;

        Self::from(cfg, block, tx)
    }

    /// Convert to MonadCfgEnv for use with MonadEvm.
    /// This applies Monad-specific defaults (128KB code size limit).
    pub fn to_monad_cfg(&self) -> MonadCfgEnv {
        MonadCfgEnv::from(self.evm_env.cfg_env.clone())
    }
}

/// Helper struct with mutable references to the block and cfg environments.
/// Generic over the spec type to support both Monad (MonadSpecId) and Ethereum (SpecId).
/// Defaults to MonadSpecId for forge/script/verify. Anvil uses SpecId explicitly.
pub struct EnvMut<'a, Spec = MonadSpecId> {
    pub block: &'a mut BlockEnv,
    pub cfg: &'a mut CfgEnv<Spec>,
    pub tx: &'a mut TxEnv,
}

impl EnvMut<'_, MonadSpecId> {
    /// Returns a copy of the environment.
    pub fn to_owned(&self) -> Env {
        Env {
            evm_env: EvmEnv { cfg_env: self.cfg.to_owned(), block_env: self.block.to_owned() },
            tx: self.tx.to_owned(),
        }
    }
}

/// Trait for getting mutable environment references.
/// Generic over the spec type to support both Monad (MonadSpecId) and Ethereum (SpecId).
pub trait AsEnvMut<Spec = MonadSpecId> {
    fn as_env_mut(&mut self) -> EnvMut<'_, Spec>;
}

impl AsEnvMut<MonadSpecId> for EnvMut<'_, MonadSpecId> {
    fn as_env_mut(&mut self) -> EnvMut<'_, MonadSpecId> {
        EnvMut { block: self.block, cfg: self.cfg, tx: self.tx }
    }
}

impl AsEnvMut<SpecId> for EnvMut<'_, SpecId> {
    fn as_env_mut(&mut self) -> EnvMut<'_, SpecId> {
        EnvMut { block: self.block, cfg: self.cfg, tx: self.tx }
    }
}

impl AsEnvMut<MonadSpecId> for Env {
    fn as_env_mut(&mut self) -> EnvMut<'_, MonadSpecId> {
        EnvMut {
            block: &mut self.evm_env.block_env,
            cfg: &mut self.evm_env.cfg_env,
            tx: &mut self.tx,
        }
    }
}

impl<DB: Database, J: JournalTr<Database = DB>, C> AsEnvMut<MonadSpecId>
    for Context<BlockEnv, TxEnv, MonadCfg, DB, J, C>
{
    fn as_env_mut(&mut self) -> EnvMut<'_, MonadSpecId> {
        EnvMut { block: &mut self.block, cfg: &mut self.cfg, tx: &mut self.tx }
    }
}

impl<DB: Database, J: JournalTr<Database = DB>, C> AsEnvMut<MonadSpecId>
    for Context<BlockEnv, TxEnv, MonadCfgEnv, DB, J, C>
{
    fn as_env_mut(&mut self) -> EnvMut<'_, MonadSpecId> {
        // MonadCfgEnv derefs to CfgEnv<MonadSpecId>
        EnvMut { block: &mut self.block, cfg: &mut *self.cfg, tx: &mut self.tx }
    }
}

// SpecId implementations for anvil compatibility (anvil uses EitherEvm pattern with multiple chains)
impl<DB: Database, J: JournalTr<Database = DB>, C> AsEnvMut<SpecId>
    for Context<BlockEnv, TxEnv, CfgEnv<SpecId>, DB, J, C>
{
    fn as_env_mut(&mut self) -> EnvMut<'_, SpecId> {
        EnvMut { block: &mut self.block, cfg: &mut self.cfg, tx: &mut self.tx }
    }
}

pub trait ContextExt {
    type DB: Database;

    fn as_db_env_and_journal(
        &mut self,
    ) -> (&mut Self::DB, &mut JournalInner<JournalEntry>, EnvMut<'_>);
}

impl<DB: Database, C> ContextExt
    for Context<BlockEnv, TxEnv, MonadCfg, DB, Journal<DB, JournalEntry>, C>
{
    type DB = DB;

    fn as_db_env_and_journal(
        &mut self,
    ) -> (&mut Self::DB, &mut JournalInner<JournalEntry>, EnvMut<'_>) {
        (
            &mut self.journaled_state.database,
            &mut self.journaled_state.inner,
            EnvMut { block: &mut self.block, cfg: &mut self.cfg, tx: &mut self.tx },
        )
    }
}

impl<DB: Database, C> ContextExt
    for Context<BlockEnv, TxEnv, MonadCfgEnv, DB, Journal<DB, JournalEntry>, C>
{
    type DB = DB;

    fn as_db_env_and_journal(
        &mut self,
    ) -> (&mut Self::DB, &mut JournalInner<JournalEntry>, EnvMut<'_>) {
        (
            &mut self.journaled_state.database,
            &mut self.journaled_state.inner,
            // MonadCfgEnv derefs to CfgEnv<MonadSpecId>
            EnvMut { block: &mut self.block, cfg: &mut *self.cfg, tx: &mut self.tx },
        )
    }
}
