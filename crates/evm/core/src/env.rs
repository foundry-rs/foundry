pub use alloy_evm::EvmEnv;
use revm::{
    context::{BlockEnv, CfgEnv, JournalTr, TxEnv},
    Context, Database,
};

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: TxEnv,
}

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
impl Env {
    pub fn from(cfg: CfgEnv, block: BlockEnv, tx: TxEnv) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx }
    }
}

/// Helper struct with references to the block and cfg environments.
pub struct EnvRef<'a> {
    pub block: &'a BlockEnv,
    pub cfg: &'a CfgEnv,
    pub tx: &'a TxEnv,
}

impl EnvRef<'_> {
    /// Returns a copy of the environment.
    pub fn to_owned(&self) -> Env {
        Env {
            evm_env: EvmEnv { cfg_env: self.cfg.to_owned(), block_env: self.block.to_owned() },
            tx: self.tx.to_owned(),
        }
    }
}

pub trait AsEnvRef {
    fn as_env_ref(&self) -> EnvRef<'_>;
}

impl AsEnvRef for EnvRef<'_> {
    fn as_env_ref(&self) -> EnvRef<'_> {
        EnvRef { block: self.block, cfg: self.cfg, tx: self.tx }
    }
}

impl AsEnvRef for Env {
    fn as_env_ref(&self) -> EnvRef<'_> {
        EnvRef { block: &self.evm_env.block_env, cfg: &self.evm_env.cfg_env, tx: &self.tx }
    }
}

impl<DB: Database, J: JournalTr<Database = DB>, C> AsEnvRef
    for Context<BlockEnv, TxEnv, CfgEnv, DB, J, C>
{
    fn as_env_ref(&self) -> EnvRef<'_> {
        EnvRef { block: &self.block, cfg: &self.cfg, tx: &self.tx }
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
