//! Optimism-specific transact helpers for the in-memory backend.

use super::{Backend, DatabaseRef};
use crate::eth::error::BlockchainError;
use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_network::Network;
use alloy_op_evm::{OpEvmContext, OpEvmFactory, OpTx};
use op_revm::{OpHaltReason, OpSpecId, OpTransaction};
use revm::{
    Inspector,
    context::{
        TxEnv,
        result::{HaltReason, ResultAndState},
    },
    database_interface::WrapDatabaseRef,
};

impl<N: Network> Backend<N> {
    /// Optimism path of [`Backend::transact_call_with_inspector_ref`].
    ///
    /// Creates an OP EVM, injects precompiles, transacts, and maps the
    /// OP-specific halt reason back to the shared [`HaltReason`].
    pub(super) fn transact_op_with_inspector_ref<'db, I>(
        &self,
        db: &'db dyn DatabaseRef,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx_env: OpTransaction<TxEnv>,
        spec: OpSpecId,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        I: Inspector<OpEvmContext<WrapDatabaseRef<&'db dyn DatabaseRef>>>,
    {
        let op_env = EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_mainnet_gas_params(spec),
            evm_env.block_env.clone(),
        );
        let mut evm = OpEvmFactory::default().create_evm_with_inspector(
            WrapDatabaseRef(db),
            op_env,
            inspector,
        );
        self.inject_configured_precompiles(evm.precompiles_mut(), evm_env);
        let result = evm.transact(OpTx(tx_env))?;
        Ok(ResultAndState {
            result: result.result.map_haltreason(|h| match h {
                OpHaltReason::Base(eth) => eth,
                OpHaltReason::FailedDeposit => HaltReason::PrecompileError,
            }),
            state: result.state,
        })
    }
}
