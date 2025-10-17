use alloy_evm::EvmEnv;
use foundry_evm::{EnvMut, core::AsEnvMut};
use foundry_evm_networks::NetworkConfigs;
use op_revm::OpTransaction;
use revm::context::{BlockEnv, CfgEnv, TxEnv};

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnd>`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: OpTransaction<TxEnv>,
    pub networks: NetworkConfigs,
}

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnv>`].
impl Env {
    pub fn new(
        cfg: CfgEnv,
        block: BlockEnv,
        tx: OpTransaction<TxEnv>,
        networks: NetworkConfigs,
    ) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx, networks }
    }
}

impl AsEnvMut for Env {
    fn as_env_mut(&mut self) -> EnvMut<'_> {
        EnvMut {
            block: &mut self.evm_env.block_env,
            cfg: &mut self.evm_env.cfg_env,
            tx: &mut self.tx.base,
        }
    }
}
