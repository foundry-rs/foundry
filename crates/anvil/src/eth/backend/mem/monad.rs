//! Monad-specific execution helpers for the in-memory backend.

use super::{
    AnvilInspector, Backend, ClientFork, DatabaseRef, EnvelopeExecutionKind, MonadExecutionContext,
};
use crate::eth::error::BlockchainError;
use alloy_consensus::BlockHeader;
use alloy_evm::{Database, Evm, EvmEnv, EvmFactory};
use alloy_monad_evm::{MonadContext, MonadEvm, MonadEvmFactory};
use alloy_network::{BlockResponse, Network};
use foundry_evm::{
    backend::DatabaseError,
    core::evm::{EvmEnvFor, FoundryEvmFactory, MonadEvmNetwork},
};
use monad_revm::{MonadChainContext, MonadHardfork, instructions::monad_gas_params};
use revm::{
    Inspector,
    context::{Transaction, TxEnv},
    context_interface::{
        result::{HaltReason, ResultAndState},
        transaction::AuthorizationTr,
    },
    database_interface::WrapDatabaseRef,
};

pub(super) struct PreparedExecution {
    pub(super) context: Option<MonadChainContext>,
    pub(super) kind: EnvelopeExecutionKind,
    pub(super) hardfork: MonadHardfork,
}

/// Caches the fork blocks needed to construct the next Monad block's ancestor context.
pub(super) async fn cache_fork_context(fork: &ClientFork) -> Result<(), BlockchainError> {
    let block_number = fork.block_number();
    let block =
        fork.block_by_number_full(block_number).await?.ok_or(BlockchainError::BlockNotFound)?;
    let parent_hash = block.header().parent_hash();
    if !parent_hash.is_zero() {
        fork.block_by_hash_full(parent_hash).await?.ok_or(BlockchainError::BlockNotFound)?;
    }
    Ok(())
}

/// Adds a candidate transaction to the current Monad block context.
fn append_transaction(chain: &mut MonadChainContext, tx: &TxEnv) {
    chain.current_tx_index = chain.current_block_senders.len();
    chain.current_block_senders.push(tx.caller());
    chain.current_block_authorities.push(
        tx.authorization_list().filter_map(|authorization| authorization.authority()).collect(),
    );
}

pub(super) fn prepare_transaction<DB: alloy_evm::Database>(
    evm: &mut MonadEvm<DB, AnvilInspector>,
    tx: &TxEnv,
) {
    append_transaction(&mut evm.ctx_mut().chain, tx);
}

pub(super) fn resolve_execution_context(
    context: Option<MonadExecutionContext<'_>>,
    tx: &TxEnv,
) -> Option<MonadChainContext> {
    match context {
        Some(MonadExecutionContext::Exact(context)) => Some(*context),
        Some(MonadExecutionContext::Next(context)) => {
            append_transaction(context, tx);
            Some(context.clone())
        }
        None => None,
    }
}

pub(super) fn advance_block(context: &mut MonadChainContext) {
    let current = context
        .current_block_senders
        .iter()
        .copied()
        .chain(context.current_block_authorities.iter().flatten().copied())
        .collect();
    context.grandparent_senders_and_authorities =
        std::mem::replace(&mut context.parent_senders_and_authorities, current);
    context.current_block_senders.clear();
    context.current_block_authorities.clear();
    context.current_tx_index = 0;
}

/// Removes a candidate transaction that failed before block inclusion.
pub(super) fn rollback_transaction<DB: alloy_evm::Database>(
    evm: &mut MonadEvm<DB, AnvilInspector>,
) {
    let chain = &mut evm.ctx_mut().chain;
    chain.current_block_senders.pop();
    chain.current_block_authorities.pop();
    chain.current_tx_index = chain.current_block_senders.len();
}

impl<N: Network> Backend<N> {
    /// Builds the Monad [`EvmEnv`] (spec and gas params) from a base env.
    pub(super) fn build_monad_evm_env(
        evm_env: &EvmEnv,
        hardfork: MonadHardfork,
    ) -> EvmEnvFor<MonadEvmNetwork> {
        EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_gas_params(hardfork, monad_gas_params(hardfork)),
            evm_env.block_env.clone(),
        )
    }

    /// Monad path of [`Backend::transact_call_with_inspector_ref`].
    pub(super) fn transact_monad_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx_env: TxEnv,
        execution: PreparedExecution,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<MonadContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        let monad_env = Self::build_monad_evm_env(evm_env, execution.hardfork);
        let factory = MonadEvmFactory::default();
        let context =
            execution.context.unwrap_or_else(|| factory.chain_context_for_transaction(&tx_env));
        let mut evm = factory.create_evm_with_inspector(WrapDatabaseRef(db), monad_env, inspector);
        evm.ctx_mut().chain = context;
        self.inject_precompiles(evm.precompiles_mut(), evm_env);
        match execution.kind {
            EnvelopeExecutionKind::Transaction => Ok(evm.transact(tx_env)?),
            EnvelopeExecutionKind::Replay => {
                if let Some(result) = factory.try_transact_system_replay(&mut evm, &tx_env)? {
                    Ok(result)
                } else {
                    Ok(evm.transact(tx_env)?)
                }
            }
        }
    }
}
