use std::fmt::Debug;

use alloy_evm::{
    eth::EthEvmContext,
    precompiles::{DynPrecompile, PrecompilesMap},
    Database, Evm,
};
use foundry_evm::backend::DatabaseError;
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
    DB: Database<Error = DatabaseError>,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
{
    for item in precompiles {
        evm.precompiles_mut()
            .apply_precompile(item.address(), |_| Some(DynPrecompile::from(*item.precompile())));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn build_evm_with_extra_precompiles() {
        // TODO: add test
    }
}
