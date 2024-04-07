use std::{fmt::Debug, sync::Arc};

use alloy_primitives::Address;
use foundry_evm::revm::{self, precompile::Precompile, ContextPrecompile, ContextPrecompiles};

/// Object-safe trait that enables injecting extra precompiles when using
/// `anvil` as a library.
pub trait PrecompileFactory: Send + Sync + Unpin + Debug {
    /// Returns a set of precompiles to extend the EVM with.
    fn precompiles(&self) -> Vec<(Address, Precompile)>;
}

/// Appends a handler register to `evm` that injects the given `precompiles`.
pub fn inject_precompiles<DB, I>(
    evm: &mut revm::Evm<'_, I, DB>,
    precompiles: Vec<(Address, Precompile)>,
) where
    DB: revm::Database,
    I: revm::Inspector<DB>,
{
    evm.handler.append_handler_register_box(Box::new(move |handler| {
        let precompiles = precompiles.clone();
        let loaded_precompiles = handler.pre_execution().load_precompiles();
        handler.pre_execution.load_precompiles = Arc::new(move || {
            let mut loaded_precompiles = loaded_precompiles.clone();
            loaded_precompiles.extend(
                precompiles
                    .clone()
                    .into_iter()
                    .map(|(addr, p)| (addr, ContextPrecompile::Ordinary(p))),
            );
            let mut default_precompiles = ContextPrecompiles::default();
            default_precompiles.extend(loaded_precompiles);
            default_precompiles
        });
    }));
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use foundry_evm::revm::{
        self,
        primitives::{address, Bytes, Precompile, PrecompileResult, SpecId},
    };

    use crate::{evm::inject_precompiles, PrecompileFactory};

    #[test]
    fn build_evm_with_extra_precompiles() {
        const PRECOMPILE_ADDR: Address = address!("0000000000000000000000000000000000000071");
        fn my_precompile(_bytes: &Bytes, _gas_limit: u64) -> PrecompileResult {
            Ok((0, Bytes::new()))
        }

        #[derive(Debug)]
        struct CustomPrecompileFactory;

        impl PrecompileFactory for CustomPrecompileFactory {
            fn precompiles(&self) -> Vec<(Address, Precompile)> {
                vec![(PRECOMPILE_ADDR, Precompile::Standard(my_precompile))]
            }
        }

        let db = revm::db::EmptyDB::default();
        let env = Box::<revm::primitives::Env>::default();
        let spec = SpecId::LATEST;
        let handler_cfg = revm::primitives::HandlerCfg::new(spec);
        let inspector = revm::inspectors::NoOpInspector;
        let context = revm::Context::new(revm::EvmContext::new_with_env(db, env), inspector);
        let handler = revm::Handler::new(handler_cfg);
        let mut evm = revm::Evm::new(context, handler);
        assert!(!evm
            .handler
            .pre_execution()
            .load_precompiles()
            .addresses()
            .any(|&addr| addr == PRECOMPILE_ADDR));

        inject_precompiles(&mut evm, CustomPrecompileFactory.precompiles());
        assert!(evm
            .handler
            .pre_execution()
            .load_precompiles()
            .addresses()
            .any(|&addr| addr == PRECOMPILE_ADDR));

        let result = evm.transact().unwrap();
        assert!(result.result.is_success());
    }
}
