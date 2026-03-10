use alloy_evm::EvmEnv;
use foundry_evm::hardfork::FoundryHardfork;
use foundry_evm::{EnvMut, core::AsEnvMut};
use foundry_evm_networks::NetworkConfigs;
use op_revm::OpTransaction;
use revm::{context::TxEnv, primitives::hardfork::SpecId};

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnd>`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: OpTransaction<TxEnv>,
    pub networks: NetworkConfigs,
    pub hardfork: FoundryHardfork,
}

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnv>`].
impl Env {
    pub fn new(
        evm_env: EvmEnv,
        tx: OpTransaction<TxEnv>,
        networks: NetworkConfigs,
        hardfork: FoundryHardfork,
    ) -> Self {
        Self { evm_env, tx, networks, hardfork }
    }
}

impl AsEnvMut<SpecId> for Env {
    fn as_env_mut(&mut self) -> EnvMut<'_, SpecId> {
        EnvMut {
            block: &mut self.evm_env.block_env,
            cfg: &mut self.evm_env.cfg_env,
            tx: &mut self.tx.base,
        }
    }
}
