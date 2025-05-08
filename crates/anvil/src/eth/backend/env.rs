use alloy_evm::EvmEnv;
use op_revm::OpTransaction;
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    primitives::hardfork::SpecId,
};

/// Helper container type for [`EvmEnv`] and [`OpTransaction<TxEnd>`].
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub evm_env: EvmEnv,
    pub tx: OpTransaction<TxEnv>,
}

/// Helper container type for [`EvmEnv`] and [`TxEnv`].
impl Env {
    pub fn default_with_spec_id(spec_id: SpecId) -> Self {
        let mut cfg = CfgEnv::default();
        cfg.spec = spec_id;

        Self::from(cfg, BlockEnv::default(), OpTransaction::default())
    }

    pub fn from(cfg: CfgEnv, block: BlockEnv, tx: OpTransaction<TxEnv>) -> Self {
        Self { evm_env: EvmEnv { cfg_env: cfg, block_env: block }, tx }
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
}
