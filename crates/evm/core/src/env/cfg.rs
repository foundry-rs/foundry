//! Configuration access and hardfork gas parameters.

use revm::{
    context::{Cfg, CfgEnv},
    primitives::hardfork::SpecId,
};
use std::fmt::Debug;

/// Access to a configuration's underlying environment and hardfork updates.
pub trait FoundryCfg:
    Cfg<Spec: Into<SpecId> + Copy + Debug> + Clone + From<CfgEnv<Self::Spec>> + Into<CfgEnv<Self::Spec>>
{
    /// Reference to the underlying configuration.
    fn cfg_env(&self) -> &CfgEnv<Self::Spec>;

    /// Mutable reference to the underlying configuration.
    fn cfg_env_mut(&mut self) -> &mut CfgEnv<Self::Spec>;

    /// Updates the hardfork and its gas parameters.
    fn set_spec_and_gas_params(&mut self, spec: Self::Spec) {
        self.cfg_env_mut().set_spec_and_mainnet_gas_params(spec);
    }
}

impl<SPEC: Into<SpecId> + Copy + Debug> FoundryCfg for CfgEnv<SPEC> {
    fn cfg_env(&self) -> &Self {
        self
    }

    fn cfg_env_mut(&mut self) -> &mut Self {
        self
    }
}

#[cfg(feature = "monad")]
impl FoundryCfg for monad_revm::MonadCfgEnv {
    fn cfg_env(&self) -> &CfgEnv<Self::Spec> {
        self.inner()
    }

    fn cfg_env_mut(&mut self) -> &mut CfgEnv<Self::Spec> {
        self.inner_mut()
    }

    fn set_spec_and_gas_params(&mut self, spec: Self::Spec) {
        self.inner_mut().spec = spec;
        self.inner_mut().set_gas_params(monad_revm::instructions::monad_gas_params(spec));
    }
}

#[cfg(all(test, feature = "monad"))]
mod tests {
    use super::*;
    use crate::env::{EvmEnv, FoundryContextExt};
    use alloy_evm::EvmFactory;
    use monad_revm::{MonadHardfork, cfg::MONAD_MEMORY_LIMIT, instructions::monad_gas_params};
    use revm::{context::BlockEnv, context_interface::ContextTr, database::EmptyDB};

    #[test]
    fn monad_memory_limit_follows_hardfork_transitions() {
        const FOUNDRY_MEMORY_LIMIT: u64 = 128 * 1024 * 1024;

        let mut cfg = CfgEnv::new_with_spec(MonadHardfork::MonadEight);
        cfg.memory_limit = FOUNDRY_MEMORY_LIMIT;
        let mut evm = alloy_monad_evm::MonadEvmFactory::default()
            .create_evm(EmptyDB::default(), EvmEnv::new(cfg, BlockEnv::default()));

        assert_eq!(evm.ctx().cfg().memory_limit(), FOUNDRY_MEMORY_LIMIT);

        for (spec, memory_limit) in [
            (MonadHardfork::MonadNine, MONAD_MEMORY_LIMIT),
            (MonadHardfork::MonadEight, FOUNDRY_MEMORY_LIMIT),
        ] {
            evm.ctx_mut().set_spec_and_gas_params(spec);
            assert_eq!(evm.ctx().cfg().inner().memory_limit, FOUNDRY_MEMORY_LIMIT);
            assert_eq!(evm.ctx().cfg().memory_limit(), memory_limit);
            assert_eq!(evm.ctx().cfg().inner().gas_params, monad_gas_params(spec));
        }
    }
}
