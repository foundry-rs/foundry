pub use alloy_evm::EvmEnv;
use revm::{
    Context, Database, Journal, JournalEntry,
    context::{BlockEnv, CfgEnv, JournalInner, JournalTr, TxEnv},
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

    /// Writes an owned [`Env`] back into the context.
    ///
    /// Counterpart to [`to_owned`](Self::to_owned): completes the read/write pair so callers
    /// that receive an updated [`Env`] by value (e.g. after a fork switch or snapshot revert)
    /// can apply it without manually assigning each field.
    pub fn set_env(&mut self, env: Env) {
        *self.block = env.evm_env.block_env;
        *self.cfg = env.evm_env.cfg_env;
        *self.tx = env.tx;
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

/// Extension trait providing mutable field access to block, tx, and cfg environments.
///
/// [`ContextTr`] only exposes immutable references for block, tx, and cfg.
/// Cheatcodes like `vm.warp()`, `vm.roll()`, `vm.chainId()` need to mutate these fields.
///
/// Also provides [`journal_and_env_mut`](FoundryContextExt::journal_and_env_mut) for
/// simultaneous mutable access to journal and env — needed because calling `journal_mut()`
/// and `block_mut()` separately would create conflicting borrows on `&mut self`.
pub trait FoundryContextExt: ContextTr {
    /// Mutable reference to the block environment.
    fn block_mut(&mut self) -> &mut BlockEnv;
    /// Mutable reference to the transaction environment.
    fn tx_mut(&mut self) -> &mut TxEnv;
    /// Mutable reference to the configuration environment.
    fn cfg_mut(&mut self) -> &mut CfgEnv;

    /// Returns a cloned snapshot of the current environment.
    fn to_env(&self) -> Env;

    /// Applies an owned [`Env`] to this context, replacing block, cfg, and tx.
    fn apply_env(&mut self, env: Env);

    /// Returns mutable references to the journal and environment simultaneously.
    ///
    /// This solves the borrow-splitting problem: calling `self.journal_mut()` and
    /// `self.block_mut()` separately would both borrow `&mut self`. This method
    /// splits the borrows at the field level in one call.
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
    fn to_env(&self) -> Env {
        Env {
            evm_env: EvmEnv { cfg_env: self.cfg.clone(), block_env: self.block.clone() },
            tx: self.tx.clone(),
        }
    }
    fn apply_env(&mut self, env: Env) {
        self.block = env.evm_env.block_env;
        self.cfg = env.evm_env.cfg_env;
        self.tx = env.tx;
    }
    fn journal_and_env_mut(&mut self) -> (&mut J, EnvMut<'_>) {
        (
            &mut self.journaled_state,
            EnvMut { block: &mut self.block, cfg: &mut self.cfg, tx: &mut self.tx },
        )
    }
}
