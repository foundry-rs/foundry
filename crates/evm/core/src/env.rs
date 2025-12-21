pub use alloy_evm::EvmEnv;
use revm::{
    Context, Database, Journal, JournalEntry,
    context::{BlockEnv, CfgEnv, JournalInner, JournalTr, TxEnv},
    primitives::hardfork::SpecId,
};

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: TxEnv,
}

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
impl Env {
    pub fn default_with_spec_id(spec_id: SpecId) -> Self {
        let mut cfg = CfgEnv::default();
        cfg.spec = spec_id;

        Self::from(cfg, BlockEnv::default(), TxEnv::default())
    }

    pub fn from(cfg: CfgEnv, block: BlockEnv, tx: TxEnv) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx }
    }

    pub fn new_with_spec_id(cfg: CfgEnv, block: BlockEnv, tx: TxEnv, spec_id: SpecId) -> Self {
        let mut cfg = cfg;
        cfg.spec = spec_id;

        Self::from(cfg, block, tx)
    }
}

/// Helper struct with mutable references to the block and cfg environments.
pub struct EnvMut<'a> {
    pub block: &'a mut BlockEnv,
    pub cfg: &'a mut CfgEnv,
    pub tx: &'a mut TxEnv,
}

impl EnvMut<'_> {
    /// Returns a copy of the environment.
    pub fn to_owned(&self) -> Env {
        Env {
            evm_env: EvmEnv { cfg_env: self.cfg.to_owned(), block_env: self.block.to_owned() },
            tx: self.tx.to_owned(),
        }
    }
}

pub trait AsEnvMut {
    fn as_env_mut(&mut self) -> EnvMut<'_>;
}

impl AsEnvMut for EnvMut<'_> {
    fn as_env_mut(&mut self) -> EnvMut<'_> {
        EnvMut { block: self.block, cfg: self.cfg, tx: self.tx }
    }
}

impl AsEnvMut for Env {
    fn as_env_mut(&mut self) -> EnvMut<'_> {
        EnvMut {
            block: &mut self.evm_env.block_env,
            cfg: &mut self.evm_env.cfg_env,
            tx: &mut self.tx,
        }
    }
}

impl<DB: Database, J: JournalTr<Database = DB>, C> AsEnvMut
    for Context<BlockEnv, TxEnv, CfgEnv, DB, J, C>
{
    fn as_env_mut(&mut self) -> EnvMut<'_> {
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
    for Context<BlockEnv, TxEnv, CfgEnv, DB, Journal<DB, JournalEntry>, C>
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
