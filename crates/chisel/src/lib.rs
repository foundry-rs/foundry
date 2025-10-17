//! Chisel is a fast, utilitarian, and verbose Solidity REPL.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate foundry_common;
#[macro_use]
extern crate tracing;

pub mod args;

pub mod cmd;

pub mod dispatcher;

pub mod executor;

pub mod opts;

pub mod runner;

pub mod session;

pub mod source;

mod solidity_helper;
pub use solidity_helper::SolidityHelper;

pub mod prelude {
    pub use crate::{cmd::*, dispatcher::*, runner::*, session::*, solidity_helper::*, source::*};
}
