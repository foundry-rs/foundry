//! Foundry trace and compiler-artifact adapter for the SolDB debugger.
//!
//! [`DebuggerBuilder::run`] opens SolDB or exports its trace and source bundle. SolDB owns the
//! debugger UI, stepping, and variable inspection; Foundry supplies execution and compiler data.
//!
//! # Migration from the built-in debugger
//!
//! Use [`DebuggerBuilder::new`] instead of `Debugger::builder`. The former `Debugger`, `DebugNode`,
//! `TUI`, frontend selection, and layout APIs have been removed. `--debug` always opens SolDB;
//! there is no optional Cargo feature or frontend flag. `--dump` now names a directory containing
//! `trace.json`, `contracts.json`, and compiler sources, rather than the old debugger JSON file.
//!
//! # Current limitations
//!
//! Profiles include forwarded call gas until gas-cost normalization is resolved in
//! <https://github.com/walnuthq/soldb/issues/200>. Root-specific profiling is tracked in
//! <https://github.com/walnuthq/soldb/issues/197>.
//! Debug metadata currently identifies one runtime per address, so source mapping after `vm.etch`
//! cannot distinguish successive runtimes at that address. Exported legacy source bundles support
//! Solidity; Vyper source maps are available only to in-process sessions. The old dump's decoded
//! precompile labels and internal-call arguments are not part of the SolDB trace schema.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;

mod builder;
pub use builder::DebuggerBuilder;

mod soldb;
