//! Base-specific transact helpers for the in-memory backend.

use super::Backend;
use crate::eth::error::BlockchainError;
use alloy_evm::{Database, Evm, EvmEnv, EvmFactory};
use alloy_network::Network;
use base_common_chains::ChainConfig;
use base_common_evm::{
    BaseContext, BaseEvmFactory, BaseHaltReason, BaseSpecId, BaseTransaction, BaseUpgrade,
};
use base_common_rpc_types::EIP8130_PRE_COBALT_RPC_ERROR;
use foundry_evm::backend::DatabaseError;
use revm::{
    DatabaseRef, Inspector,
    context::{
        TxEnv,
        result::{HaltReason, ResultAndState},
    },
    database_interface::WrapDatabaseRef,
};

impl<N: Network> Backend<N> {
    /// Base path of [`Backend::transact_call_with_inspector_ref`].
    pub(super) fn transact_base_with_inspector_ref<'db, I, DB>(
        &self,
        db: &'db DB,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx: BaseTransaction<TxEnv>,
    ) -> Result<ResultAndState<HaltReason>, BlockchainError>
    where
        DB: DatabaseRef + ?Sized,
        I: Inspector<BaseContext<WrapDatabaseRef<&'db DB>>>,
        WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
    {
        let upgrade = self.base_upgrade_at_timestamp(evm_env.block_env.timestamp.saturating_to());
        if tx.eip8130.is_some() && upgrade < BaseUpgrade::Cobalt {
            return Err(BlockchainError::InvalidTransactionRequest(
                EIP8130_PRE_COBALT_RPC_ERROR.to_string(),
            ));
        }
        let base_env = EvmEnv::new(
            evm_env.cfg_env.clone().with_spec_and_mainnet_gas_params(BaseSpecId::new(upgrade)),
            evm_env.block_env.clone(),
        );
        let activation_admin = self.base_activation_admin().or_else(|| {
            ChainConfig::activation_admin_address_for_upgrade_by_chain_id(
                base_env.cfg_env.chain_id,
                upgrade,
            )
        });
        let factory = BaseEvmFactory::new(activation_admin);
        let mut evm = factory.create_evm_with_inspector(WrapDatabaseRef(db), base_env, inspector);
        evm.ctx_mut().cfg.tx_chain_id_check = true;
        self.inject_precompiles(evm.precompiles_mut(), evm_env);
        let result = Evm::transact_raw(&mut evm, tx)?;
        Ok(ResultAndState {
            result: result.result.map_haltreason(|halt| match halt {
                BaseHaltReason::Base(eth) => eth,
                BaseHaltReason::FailedDeposit => HaltReason::PrecompileError,
            }),
            state: result.state,
        })
    }
}
