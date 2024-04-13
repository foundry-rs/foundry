//! # foundry-evm-core
//!
//! Core EVM abstractions.

#![warn(unused_crate_dependencies)]

use auto_impl::auto_impl;
use revm::{inspectors::NoOpInspector, interpreter::CreateInputs, Database, EvmContext, Inspector};
use revm_inspectors::access_list::AccessListInspector;

#[macro_use]
extern crate tracing;

mod ic;

pub mod abi;
pub mod backend;
pub mod constants;
pub mod debug;
pub mod decode;
pub mod fork;
pub mod opcodes;
pub mod opts;
pub mod snapshot;
pub mod utils;

#[auto_impl(&mut, Box)]
pub trait InspectorExt<DB: Database>: Inspector<DB> {
    fn should_use_create2_factory(
        &mut self,
        _context: &mut EvmContext<DB>,
        _inputs: &mut CreateInputs,
    ) -> bool {
        false
    }
}

impl<DB: Database> InspectorExt<DB> for NoOpInspector {}
impl<DB: Database> InspectorExt<DB> for AccessListInspector {}
