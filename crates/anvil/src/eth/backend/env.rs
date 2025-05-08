use alloy_evm::EvmEnv;
use foundry_evm::EnvMut;
use foundry_evm_core::AsEnvMut;
use op_revm::{transaction::deposit::DepositTransactionParts, OpTransaction};
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    primitives::hardfork::SpecId,
};

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnd>`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: OpTransaction<TxEnv>,
    pub is_optimism: bool,
}

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnv>`].
impl Env {
    pub fn default_with_spec_id(spec_id: SpecId) -> Self {
        let mut cfg = CfgEnv::default();
        cfg.spec = spec_id;

        Self::from(cfg, BlockEnv::default(), OpTransaction::default())
    }

    pub fn from(cfg: CfgEnv, block: BlockEnv, tx: OpTransaction<TxEnv>) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx, is_optimism: false }
    }

    pub fn new_with_spec_id(
        cfg: CfgEnv,
        block: BlockEnv,
        tx: OpTransaction<TxEnv>,
        spec_id: SpecId,
    ) -> Self {
        let mut cfg = cfg;
        cfg.spec = spec_id;

        Self::from(cfg, block, tx)
    }

    pub fn with_deposit(mut self, deposit: DepositTransactionParts) -> Self {
        self.tx.deposit = deposit;
        self
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
