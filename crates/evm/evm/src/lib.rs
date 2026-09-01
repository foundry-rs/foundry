//! # foundry-evm
//!
//! Main Foundry EVM backend abstractions.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate tracing;

pub mod executors;
pub mod inspectors;

pub use foundry_evm_core as core;
pub use foundry_evm_core::{
    EvmEnv, FoundryInspectorExt, InspectorExt, backend, constants, decode, fork, hardfork, opts,
    utils,
};
pub use foundry_evm_coverage as coverage;
pub use foundry_evm_fuzz as fuzz;
pub use foundry_evm_hardforks as hardforks;
pub use foundry_evm_traces as traces;

/// Dispatches an expression to the EVM network selected by a [`NetworkConfigs`]-like value.
///
/// The identifier between pipes is bound to the selected [`FoundryEvmNetwork`] type within the
/// expression.
///
/// [`FoundryEvmNetwork`]: core::evm::FoundryEvmNetwork
/// [`NetworkConfigs`]: foundry_evm_networks::NetworkConfigs
#[macro_export]
macro_rules! dispatch_evm_network {
    ($networks:expr, | $network:ident | $body:expr) => {{
        let networks = &$networks;
        match () {
            _ if networks.is_tempo() => {
                type $network = $crate::core::evm::TempoEvmNetwork;
                $body
            }
            #[cfg(feature = "monad")]
            _ if networks.is_monad() => {
                type $network = $crate::core::evm::MonadEvmNetwork;
                $body
            }
            #[cfg(feature = "optimism")]
            _ if networks.is_optimism() => {
                type $network = $crate::core::evm::OpEvmNetwork;
                $body
            }
            _ => {
                type $network = $crate::core::evm::EthEvmNetwork;
                $body
            }
        }
    }};
}

// TODO: We should probably remove these, but it's a pretty big breaking change.
#[doc(hidden)]
pub use revm;
