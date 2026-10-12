//! # foundry-evm-core
//!
//! Ethereum execution state, options, and fork sources shared by Foundry tools.
//!
//! Native Ethereum uses [`ethereum`] for EVM2-owned state and RPC backing. The `revm` feature
//! retains the legacy executor, environment, and network factory adapters.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use alloy_primitives::{Address, map::HashMap};

#[cfg(feature = "revm")]
use crate::constants::DEFAULT_CREATE2_DEPLOYER;
#[cfg(feature = "revm")]
use auto_impl::auto_impl;
#[cfg(feature = "revm")]
use foundry_evm_networks::NetworkConfigs;
#[cfg(feature = "revm")]
use revm::{Inspector, inspector::NoOpInspector, interpreter::CreateInputs};
#[cfg(feature = "revm")]
use revm_inspectors::access_list::AccessListInspector;

#[cfg(feature = "optimism")]
use op_alloy_rpc_types as _;

/// Map keyed by breakpoints char to their location (contract address, pc)
pub type Breakpoints = HashMap<char, (Address, usize)>;

#[macro_use]
extern crate tracing;

pub mod abi {
    pub use foundry_cheatcodes_spec::Vm;
    pub use foundry_evm_abi::*;
}

#[cfg(feature = "revm")]
pub mod env;
#[cfg(feature = "revm")]
pub use env::*;

pub mod ethereum;

#[cfg(feature = "revm")]
pub mod backend;
pub mod buffer;
pub mod bytecode;
pub mod constants;
pub mod decode;
pub mod eip2935;
pub mod evm;
pub mod fork;
pub mod hardfork;
pub mod ic;
pub mod opts;
pub mod precompiles;
pub mod state_snapshot;
#[cfg(feature = "revm")]
pub mod tempo;
pub mod utils;

/// Foundry-specific inspector methods, decoupled from any particular EVM context type.
///
/// This trait holds Foundry-specific extensions (create2 factory, console logging, temporary Celo
/// configuration, deployer address). It has no `Inspector<CTX>` supertrait so it can
/// be used in generic code with `I: FoundryInspectorExt + Inspector<CTX>`.
#[cfg(feature = "revm")]
#[auto_impl(&mut, Box)]
pub trait InspectorExt {
    /// Determines whether the `DEFAULT_CREATE2_DEPLOYER` should be used for a CREATE2 frame.
    ///
    /// If this function returns true, we'll replace CREATE2 frame with a CALL frame to CREATE2
    /// factory.
    fn should_use_create2_factory(&mut self, _depth: usize, _inputs: &CreateInputs) -> bool {
        false
    }

    /// Simulates `console.log` invocation.
    fn console_log(&mut self, msg: &str) {
        let _ = msg;
    }

    /// Returns configuration retained for Celo precompile support.
    fn get_networks(&self) -> NetworkConfigs {
        NetworkConfigs::default()
    }

    /// Returns the CREATE2 deployer address.
    fn create2_deployer(&self) -> Address {
        DEFAULT_CREATE2_DEPLOYER
    }
}

/// A combined inspector trait that integrates revm's [`Inspector`] with Foundry-specific
/// extensions. Automatically implemented for any type that implements both [`Inspector<CTX>`]
/// and [`InspectorExt`].
#[cfg(feature = "revm")]
pub trait FoundryInspectorExt<CTX: FoundryContextExt>: Inspector<CTX> + InspectorExt {}

#[cfg(feature = "revm")]
impl<CTX: FoundryContextExt, T> FoundryInspectorExt<CTX> for T where T: Inspector<CTX> + InspectorExt
{}

#[cfg(feature = "revm")]
impl InspectorExt for NoOpInspector {}

#[cfg(feature = "revm")]
impl InspectorExt for AccessListInspector {}
