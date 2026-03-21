use alloy_evm::EvmEnv;
use foundry_evm_networks::NetworkConfigs;
use op_revm::OpTransaction;
use revm::context::TxEnv;

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnd>`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: OpTransaction<TxEnv>,
    pub networks: NetworkConfigs,
}

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnv>`].
impl Env {
    pub fn new(evm_env: EvmEnv, tx: OpTransaction<TxEnv>, networks: NetworkConfigs) -> Self {
        Self { evm_env, tx, networks }
    }
}
