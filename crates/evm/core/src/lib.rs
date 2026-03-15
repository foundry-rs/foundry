//! # foundry-evm-core
//!
//! Core EVM abstractions.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

use crate::constants::DEFAULT_CREATE2_DEPLOYER;
use alloy_evm::eth::EthEvmContext;
use alloy_primitives::{Address, map::HashMap};
use auto_impl::auto_impl;
use backend::DatabaseExt;
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
pub mod either_evm;
pub mod evm;
pub mod fork;
pub mod hardfork;
pub mod ic;
pub mod opts;
pub mod precompiles;
pub mod state_snapshot;
pub mod utils;

/// Foundry-specific inspector methods, decoupled from any particular EVM context type.
///
/// This trait holds Foundry-specific extensions (create2 factory, console logging,
/// network config, deployer address). It has no `Inspector<CTX>` supertrait so it can
/// be used in generic code with `I: FoundryInspectorExt + Inspector<CTX>`.
#[auto_impl(&mut, Box)]
pub trait FoundryInspectorExt {
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

    /// Returns configured networks.
    fn get_networks(&self) -> NetworkConfigs {
        NetworkConfigs::default()
    }

    /// Returns the CREATE2 deployer address.
    fn create2_deployer(&self) -> Address {
        DEFAULT_CREATE2_DEPLOYER
    }

    /// Returns a mutable reference to the concrete type as `dyn Any`, for downcasting.
    ///
    /// Used to recover the concrete inspector type (e.g. `InspectorStack`) from a
    /// `&mut dyn FoundryInspectorExt` trait object.
    ///
    /// Returns `None` for types that cannot be downcasted (e.g. non-`'static` borrows).
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any>;
}

/// Convenience extension for downcasting `dyn FoundryInspectorExt` to concrete types.
pub trait FoundryInspectorDowncastExt: FoundryInspectorExt {
    /// Attempts to downcast to a concrete type `T`.
    fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.as_any_mut()?.downcast_mut::<T>()
    }
}

impl<I: FoundryInspectorExt + ?Sized> FoundryInspectorDowncastExt for I {}

/// Combined trait: `Inspector<EthEvmContext<...>>` + [`FoundryInspectorExt`].
///
/// Used as a trait object (`dyn InspectorExt`) in backend code that is Eth-specific.
/// For generic multi-network code, use `I: FoundryInspectorExt + Inspector<CTX>` instead.
pub trait InspectorExt:
    for<'a> Inspector<EthEvmContext<&'a mut dyn DatabaseExt>> + FoundryInspectorExt
{
}

impl<T> InspectorExt for T where
    T: for<'a> Inspector<EthEvmContext<&'a mut dyn DatabaseExt>> + FoundryInspectorExt
{
}

impl FoundryInspectorExt for NoOpInspector {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl FoundryInspectorExt for AccessListInspector {
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
