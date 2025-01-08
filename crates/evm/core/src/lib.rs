//! # foundry-evm-core
//!
//! Core EVM abstractions.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use crate::constants::DEFAULT_CREATE2_DEPLOYER;
use alloy_primitives::Address;
use auto_impl::auto_impl;
use backend::DatabaseExt;
use revm::{inspectors::NoOpInspector, interpreter::CreateInputs, EvmContext, Inspector};
use revm_inspectors::access_list::AccessListInspector;

#[macro_use]
extern crate tracing;

pub mod abi {
    pub use foundry_cheatcodes_spec::Vm;
    pub use foundry_evm_abi::*;
}

mod ic;

pub mod backend;
pub mod buffer;
pub mod constants;
pub mod decode;
pub mod fork;
pub mod opcodes;
pub mod opts;
pub mod precompiles;
pub mod state_snapshot;
pub mod utils;

/// An extension trait that allows us to add additional hooks to Inspector for later use in
/// handlers.
#[auto_impl(&mut, Box)]
pub trait InspectorExt: for<'a> Inspector<&'a mut dyn DatabaseExt> {
    /// Determines whether the `DEFAULT_CREATE2_DEPLOYER` should be used for a CREATE2 frame.
    ///
    /// If this function returns true, we'll replace CREATE2 frame with a CALL frame to CREATE2
    /// factory.
    fn should_use_create2_factory(
        &mut self,
        _context: &mut EvmContext<&mut dyn DatabaseExt>,
        _inputs: &mut CreateInputs,
    ) -> bool {
        false
    }

    /// Simulates `console.log` invocation.
    fn console_log(&mut self, _input: String) {}

    /// Returns `true` if the current network is Odyssey.
    fn is_odyssey(&self) -> bool {
        false
    }

    /// Returns the CREATE2 deployer address.
    fn create2_deployer(&self) -> Address {
        DEFAULT_CREATE2_DEPLOYER
    }
}

impl InspectorExt for NoOpInspector {}

impl InspectorExt for AccessListInspector {}
