use std::fmt::Debug;

use alloy_evm::{eth::EthEvmContext, precompiles::PrecompilesMap, Database, Evm};
use foundry_evm::backend::DatabaseError;
use foundry_evm_core::either_evm::EitherEvm;
use op_revm::OpContext;
use revm::{precompile::Precompiles, Inspector};

/// Object-safe trait that enables injecting extra precompiles when using
/// `anvil` as a library.
pub trait PrecompileFactory: Send + Sync + Unpin + Debug {
    /// Returns a set of precompiles to extend the EVM with.
    fn precompiles(&self) -> Precompiles;
}

/// Appends a handler register to `evm` that injects the given `precompiles`.
///
/// This will add an additional handler that extends the default precompiles with the given set of
/// precompiles.
pub fn inject_precompiles<DB, I>(
    evm: &mut EitherEvm<DB, I, PrecompilesMap>,
    precompiles: Precompiles,
) where
    DB: Database<Error = DatabaseError>,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
{
    for (precompile_addr, precompile) in precompiles.inner().iter() {
        evm.precompiles_mut().apply_precompile(precompile_addr, precompile);
    }

    // let precompiles = evm.precompiles_mut();
    // evm.precompiles_mut().extend(precompiles.clone());

    // evm.handler.append_handler_register_box(Box::new(move |handler| {
    //     let precompiles = precompiles.clone();
    //     let prev = handler.pre_execution.load_precompiles.clone();
    //     handler.pre_execution.load_precompiles = Arc::new(move || {
    //         let mut cx = prev();
    //         cx.extend(precompiles.iter().cloned().map(|(a, b)| (a, b.into())));
    //         cx
    //     });
    // }));
}

// #[cfg(test)]
// mod tests {
//     use crate::{evm::inject_precompiles, PrecompileFactory};
//     use alloy_primitives::{address, Address, Bytes};
//     use revm::{
//         precompile::{PrecompileOutput, PrecompileResult, Precompiles},
//         primitives::hardfork::SpecId,
//     };

//     #[test]
//     fn build_evm_with_extra_precompiles() {
//         const PRECOMPILE_ADDR: Address = address!("0x0000000000000000000000000000000000000071");

//         fn my_precompile(_bytes: &Bytes, _gas_limit: u64) -> PrecompileResult {
//             Ok(PrecompileOutput { bytes: Bytes::new(), gas_used: 0 })
//         }

//         #[derive(Debug)]
//         struct CustomPrecompileFactory;

//         impl PrecompileFactory for CustomPrecompileFactory {
//             fn precompiles(&self) -> Vec<(Address, Precompile)> {
//                 vec![(PRECOMPILE_ADDR, Precompile::Standard(my_precompile))]
//             }
//         }

//         let db = revm::db::EmptyDB::default();
//         let env = Box::<revm::primitives::Env>::default();
//         let spec = SpecId::default();
//         let handler_cfg = revm::primitives::HandlerCfg::new(spec);
//         let inspector = revm::inspectors::NoOpInspector;
//         let context = revm::Context::new(revm::EvmContext::new_with_env(db, env), inspector);
//         let handler = revm::Handler::new(handler_cfg);
//         let mut evm = revm::Evm::new(context, handler);
//         assert!(!evm
//             .handler
//             .pre_execution()
//             .load_precompiles()
//             .addresses()
//             .any(|&addr| addr == PRECOMPILE_ADDR));

//         inject_precompiles(&mut evm, CustomPrecompileFactory.precompiles());
//         assert!(evm
//             .handler
//             .pre_execution()
//             .load_precompiles()
//             .addresses()
//             .any(|&addr| addr == PRECOMPILE_ADDR));

//         let result = evm.transact().unwrap();
//         assert!(result.result.is_success());
//     }
// }
