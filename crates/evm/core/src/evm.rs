use std::fmt::Debug;

use crate::{
    Env, EvmEnv, FoundryContextExt, FoundryInspectorExt,
    backend::{DatabaseExt, FoundryJournalExt, JournaledState},
};
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        TxEnv,
        result::{EVMError, HaltReason, ResultAndState},
    },
    handler::FrameResult,
    interpreter::FrameInput,
};

/// Closure type for [`FoundryEvmFactory::call_nested`] and [`CheatcodesExecutor`] methods
/// that run nested EVM operations.
pub type NestedEvmClosure<'a> =
    &'a mut dyn FnMut(&mut dyn NestedEvm) -> Result<(), EVMError<DatabaseError>>;

/// Factory trait for constructing and running network-specific EVMs.
///
/// This abstracts over the concrete EVM type (`EthEvm` for Eth, `TempoEvm` for Tempo, etc.)
/// so that `Backend` and `CowBackend` don't need to know which network's EVM they're using.
///
/// The inspector parameter is `&mut dyn FoundryInspectorExt` — network-agnostic. Each factory
/// implementation uses [`FoundryInspectorExt::as_any_mut`] to downcast to its network-specific
/// concrete inspector type (e.g. `InspectorStack` for Eth).
pub trait FoundryEvmFactory: Debug + Send + Sync {
    /// Creates a network-specific EVM, runs a transaction, and returns the result.
    ///
    /// The `env` is updated in-place with any changes the EVM made (e.g. cheatcodes modifying
    /// block/tx fields).
    fn inspect(
        &self,
        db: &mut dyn DatabaseExt,
        env: &mut Env,
        inspector: &mut dyn FoundryInspectorExt,
    ) -> eyre::Result<ResultAndState>;

    /// Like [`inspect`](Self::inspect) but sets the journal depth before transacting.
    ///
    /// Used by `transact_from_tx` and `commit_transaction` which replay historical transactions
    /// inside an already-running execution context.
    fn transact_with_depth(
        &self,
        db: &mut dyn DatabaseExt,
        env: Env,
        inspector: &mut dyn FoundryInspectorExt,
        depth: usize,
    ) -> eyre::Result<ResultAndState>;

    /// Builds a network-specific EVM, passes it to the closure as `&mut dyn NestedEvm`,
    /// then extracts and returns the environment and journal state.
    ///
    /// This is the lower-level primitive used by `CheatcodesExecutor::with_nested_evm` and
    /// `with_fresh_nested_evm`. The caller handles context cloning and state writeback;
    /// this method only handles EVM construction, closure execution, and state extraction.
    fn call_nested(
        &self,
        db: &mut dyn DatabaseExt,
        env: Env,
        inspector: &mut dyn FoundryInspectorExt,
        f: NestedEvmClosure<'_>,
    ) -> eyre::Result<(Env, JournaledState)>;
}

/// Object-safe trait exposing the operations that cheatcode nested EVM closures need.
///
/// This abstracts over the concrete EVM type (`EthEvm`, future `TempoEvm`, etc.)
/// so that cheatcode impls can build and run nested EVMs without knowing the concrete type.
pub trait NestedEvm {
    /// Returns a mutable reference to the journal inner state (`JournaledState`).
    fn journal_inner_mut(&mut self) -> &mut JournaledState;

    /// Runs a single execution frame (create or call) through the EVM handler loop.
    fn run_execution(&mut self, frame: FrameInput) -> Result<FrameResult, EVMError<DatabaseError>>;

    /// Executes a full transaction with the given `TxEnv`.
    fn transact(
        &mut self,
        tx: TxEnv,
    ) -> Result<ResultAndState<HaltReason>, EVMError<DatabaseError>>;

    /// Returns a snapshot of the current environment (cfg + block, tx).
    fn to_env(&self) -> (EvmEnv, TxEnv);

    /// Takes the journal inner state out, replacing it with default.
    fn take_journal_inner(&mut self) -> JournaledState;
}

/// Clones the current context (env + journal), passes the database, cloned env,
/// and cloned journal inner to the callback. The callback builds whatever EVM it
/// needs, runs its operations, and returns `(result, modified_env, modified_journal)`.
/// Modified state is written back after the callback returns.
pub fn with_cloned_context<CTX: FoundryContextExt, R>(
    ecx: &mut CTX,
    f: impl FnOnce(
        &mut dyn DatabaseExt,
        EvmEnv,
        TxEnv,
        JournaledState,
    ) -> Result<(R, EvmEnv, TxEnv, JournaledState), EVMError<DatabaseError>>,
) -> Result<R, EVMError<DatabaseError>>
where
    CTX::Journal: FoundryJournalExt,
{
    let (evm_env, tx_env) = Env::clone_evm_and_tx(ecx);

    let journal = ecx.journal_mut();
    let (db, journal_inner) = journal.as_db_and_inner();
    let journal_inner_clone = journal_inner.clone();

    let (result, sub_evm_env, sub_tx, sub_inner) = f(db, evm_env, tx_env, journal_inner_clone)?;

    // Write back modified state. The db borrow was released when f returned.
    ecx.journal_mut().set_inner(sub_inner);
    Env::apply_evm_and_tx(ecx, sub_evm_env, sub_tx);

    Ok(result)
}
