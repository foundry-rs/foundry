//! Optimism-specific transact helpers for the in-memory backend.

use super::Backend;
use crate::eth::error::BlockchainError;
use alloy_evm::{Database, Evm, EvmEnv, EvmFactory};
use alloy_network::Network;
use alloy_op_evm::{OpEvmContext, OpEvmFactory, OpTx};
use foundry_evm::backend::DatabaseError;
use op_revm::{OpHaltReason, OpSpecId, OpTransaction};
use revm::{
    DatabaseRef, Inspector,
    context::{
        TxEnv,
        result::{EVMError, HaltReason, ResultAndState},
    },
    database_interface::WrapDatabaseRef,
};

impl<N: Network> Backend<N> {
    /// Optimism path of [`Backend::transact_with_inspector_ref`].
    ///
    /// Creates an OP EVM, injects precompiles, transacts, and maps the
    /// OP-specific halt reason back to the shared [`HaltReason`].
    pub(super) fn transact_op_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx_env: OpTransaction<TxEnv>,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<OpEvmContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        let op_env = EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_mainnet_gas_params(OpSpecId::ISTHMUS),
            evm_env.block_env.clone(),
        );
        let mut evm = OpEvmFactory::default().create_evm_with_inspector(
            WrapDatabaseRef(db),
            op_env,
            inspector,
        );
        self.inject_precompiles(evm.precompiles_mut());
        let result = evm.transact(OpTx(tx_env)).map_err(|e| match e {
            EVMError::Database(db) => EVMError::Database(db),
            EVMError::Header(h) => EVMError::Header(h),
            EVMError::Custom(s) => EVMError::Custom(s),
            EVMError::CustomAny(err) => EVMError::CustomAny(err),
            EVMError::Transaction(t) => EVMError::Transaction(t),
        })?;
        Ok(ResultAndState {
            result: result.result.map_haltreason(|h| match h {
                OpHaltReason::Base(eth) => eth,
                _ => HaltReason::PrecompileError,
            }),
            state: result.state,
        })
    }
}
