pub use alloy_evm::EvmEnv;
use revm::{
    Context, Database,
    context::{BlockEnv, CfgEnv, JournalTr, TxEnv},
    context_interface::ContextTr,
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

/// Extension trait providing mutable field access to block/tx/cfg.
///
/// Needed because [`ContextTr::all_mut()`] only returns immutable references for
/// block, tx, and cfg. Cheatcodes like `vm.warp()`, `vm.roll()`, `vm.chainId()`
/// need to mutate these fields.
pub trait FoundryContextExt: ContextTr<Block = BlockEnv, Tx = TxEnv, Cfg = CfgEnv> {
    /// Get a mutable reference to the block environment.
    fn block_mut(&mut self) -> &mut BlockEnv;
    /// Get a mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut TxEnv;
    /// Get a mutable reference to the configuration environment.
    fn cfg_mut(&mut self) -> &mut CfgEnv;

    /// Returns a cloned snapshot of the current environment.
    fn to_env(&self) -> Env {
        Env {
            evm_env: EvmEnv { cfg_env: self.cfg().clone(), block_env: self.block().clone() },
            tx: self.tx().clone(),
        }
    }

    /// Applies an owned [`Env`] to this context, replacing block, cfg, and tx.
    fn apply_env(&mut self, env: Env) {
        *self.block_mut() = env.evm_env.block_env;
        *self.cfg_mut() = env.evm_env.cfg_env;
        *self.tx_mut() = env.tx;
    }

    /// Returns mutable references to the journal and environment simultaneously.
    ///
    /// This enables the caller to hold `&mut Journal` and `&mut EnvMut` at the same time,
    /// which is needed for `DatabaseExt` methods that require both.
    /// A single `&mut self` call avoids the borrow-splitting problem that arises
    /// when calling `journal_mut()` and `block_mut()`/`tx_mut()`/`cfg_mut()` separately.
    fn journal_and_env_mut(&mut self) -> (&mut Self::Journal, EnvMut<'_>);
}

impl<DB: Database, J: JournalTr<Database = DB>, C> FoundryContextExt
    for Context<BlockEnv, TxEnv, CfgEnv, DB, J, C>
{
    fn block_mut(&mut self) -> &mut BlockEnv {
        &mut self.block
    }
    fn tx_mut(&mut self) -> &mut TxEnv {
        &mut self.tx
    }
    fn cfg_mut(&mut self) -> &mut CfgEnv {
        &mut self.cfg
    }
    fn journal_and_env_mut(&mut self) -> (&mut J, EnvMut<'_>) {
        (
            &mut self.journaled_state,
            EnvMut { block: &mut self.block, cfg: &mut self.cfg, tx: &mut self.tx },
        )
    }
}
