//! # foundry-evm-core
//!
//! Generic execution, environment, fork, backend, and state abstractions shared by Foundry tools.
//!
//! [`evm::FoundryEvmNetwork`] binds an Alloy network to an [`evm::FoundryEvmFactory`]. The factory
//! owns the concrete execution types and constructs a Foundry-compatible EVM context, while
//! `foundry-evm-networks` owns runtime family selection. Keeping those responsibilities separate
//! allows one compiled binary to dispatch to different execution families at runtime.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "optimism")]
use op_alloy_rpc_types as _;

use crate::constants::DEFAULT_CREATE2_DEPLOYER;
use alloy_primitives::{Address, map::HashMap};
use auto_impl::auto_impl;
use revm::{Inspector, inspector::NoOpInspector, interpreter::CreateInputs};
use revm_inspectors::access_list::AccessListInspector;

/// Map keyed by breakpoints char to their location (contract address, pc)
pub type Breakpoints = HashMap<char, (Address, usize)>;

#[macro_use]
extern crate tracing;

pub mod abi {
    pub use foundry_cheatcodes_spec::Vm;
    pub use foundry_evm_abi::*;
}

pub mod env;
pub use env::*;
use foundry_evm_networks::NetworkConfigs;

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
pub mod tempo;
pub mod utils;

/// Foundry-specific inspector methods, decoupled from any particular EVM context type.
///
/// This trait holds Foundry-specific extensions (create2 factory, console logging, temporary Celo
/// configuration, deployer address). It has no `Inspector<CTX>` supertrait so it can
/// be used in generic code with `I: FoundryInspectorExt + Inspector<CTX>`.
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
pub trait FoundryInspectorExt<CTX: FoundryContextExt>: Inspector<CTX> + InspectorExt {}

impl<CTX: FoundryContextExt, T> FoundryInspectorExt<CTX> for T where T: Inspector<CTX> + InspectorExt
{}

impl InspectorExt for NoOpInspector {}

impl InspectorExt for AccessListInspector {}
