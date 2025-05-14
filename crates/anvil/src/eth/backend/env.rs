use alloy_evm::EvmEnv;
use foundry_evm::EnvMut;
use foundry_evm_core::AsEnvMut;
use op_revm::OpTransaction;
use revm::context::{BlockEnv, CfgEnv, TxEnv};

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnd>`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: OpTransaction<TxEnv>,
    pub is_optimism: bool,
}

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnv>`].
impl Env {
    pub fn new(cfg: CfgEnv, block: BlockEnv, tx: OpTransaction<TxEnv>, is_optimism: bool) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx, is_optimism }
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
