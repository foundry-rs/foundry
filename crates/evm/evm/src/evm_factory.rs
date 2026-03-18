use alloy_evm::{Evm, EvmEnv};
use eyre::WrapErr;
use foundry_evm_core::{
    backend::DatabaseExt,
    evm::{FoundryEvmFactory, new_evm_with_inspector},
};
use revm::{
    context::{BlockEnv, TxEnv},
    context_interface::result::{HaltReason, ResultAndState},
    primitives::hardfork::SpecId,
};

use crate::inspectors::InspectorStack;

/// Ethereum-flavoured [`FoundryEvmFactory`].
#[derive(Debug, Clone, Default)]
pub struct EthEvmFactory;

impl FoundryEvmFactory for EthEvmFactory {
    type Inspector = InspectorStack;
    type Spec = SpecId;
    type Block = BlockEnv;
    type Tx = TxEnv;
    type HaltReason = HaltReason;

    fn inspect(
        &self,
        db: &mut dyn DatabaseExt,
        evm_env: &mut EvmEnv,
        tx_env: &mut TxEnv,
        inspector: &mut InspectorStack,
    ) -> eyre::Result<ResultAndState> {
        let mut evm = new_evm_with_inspector(db, evm_env.clone(), tx_env.clone(), &mut *inspector);

        let res = evm.transact(tx_env.clone()).wrap_err("EVM error")?;

        *evm_env = EvmEnv::new(evm.cfg.clone(), evm.block.clone());
        *tx_env = evm.tx.clone();

        Ok(res)
    }
}
