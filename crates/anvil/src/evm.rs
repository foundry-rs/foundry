use std::fmt::Debug;

use alloy_evm::{
    eth::EthEvmContext,
    precompiles::{DynPrecompile, PrecompilesMap},
    Database, Evm,
};
use foundry_evm_core::either_evm::EitherEvm;
use op_revm::OpContext;
use revm::{precompile::PrecompileWithAddress, Inspector};

/// Object-safe trait that enables injecting extra precompiles when using
/// `anvil` as a library.
pub trait PrecompileFactory: Send + Sync + Unpin + Debug {
    /// Returns a set of precompiles to extend the EVM with.
    fn precompiles(&self) -> Vec<PrecompileWithAddress>;
}

/// Inject precompiles into the EVM dynamically.
pub fn inject_precompiles<DB, I>(
    evm: &mut EitherEvm<DB, I, PrecompilesMap>,
    precompiles: Vec<PrecompileWithAddress>,
) where
    DB: Database,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
{
    for p in precompiles {
        evm.precompiles_mut()
            .apply_precompile(p.address(), |_| Some(DynPrecompile::from(*p.precompile())));
    }
}

#[cfg(test)]
mod tests {
    use alloy_evm::{eth::EthEvmContext, precompiles::PrecompilesMap, EthEvm, Evm};
    use alloy_primitives::{address, Address, Bytes};
    use foundry_evm::Env;
    use foundry_evm_core::either_evm::EitherEvm;
    use itertools::Itertools;
    use revm::{
        context::{Evm as RevmEvm, JournalTr, LocalContext},
        database::EmptyDB,
        handler::{instructions::EthInstructions, EthPrecompiles},
        inspector::NoOpInspector,
        interpreter::interpreter::EthInterpreter,
        precompile::{PrecompileOutput, PrecompileResult, PrecompileWithAddress},
        Journal,
    };

    use crate::{inject_precompiles, PrecompileFactory};

    #[test]
    fn build_evm_with_extra_precompiles() {
        const PRECOMPILE_ADDR: Address = address!("0x0000000000000000000000000000000000000071");

        fn my_precompile(_input: &[u8], _gas_limit: u64) -> PrecompileResult {
            Ok(PrecompileOutput { bytes: Bytes::new(), gas_used: 0 })
        }

        #[derive(Debug)]
        struct CustomPrecompileFactory;

        impl PrecompileFactory for CustomPrecompileFactory {
            fn precompiles(&self) -> Vec<PrecompileWithAddress> {
                vec![PrecompileWithAddress::from((
                    PRECOMPILE_ADDR,
                    my_precompile as fn(&[u8], u64) -> PrecompileResult,
                ))]
            }
        }

        let env = Env::default();
        let evm_context = EthEvmContext {
            journaled_state: Journal::new(EmptyDB::default()),
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone(),
            tx: env.tx.clone(),
            chain: (),
            local: LocalContext::default(),
            error: Ok(()),
        };

        let mut evm = EitherEvm::Eth(EthEvm::new(
            RevmEvm::new_with_inspector(
                evm_context,
                NoOpInspector,
                EthInstructions::<EthInterpreter, EthEvmContext<EmptyDB>>::default(),
                PrecompilesMap::from_static(EthPrecompiles::default().precompiles),
            ),
            true,
        ));

        assert!(!evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        inject_precompiles(&mut evm, CustomPrecompileFactory.precompiles());

        assert!(evm.precompiles_mut().addresses().contains(&PRECOMPILE_ADDR));

        let result = match &mut evm {
            EitherEvm::Eth(evm_eth) => evm_eth.transact(env.tx).unwrap(),
            _ => unreachable!(),
        };
        assert!(result.result.is_success());
    }
}
