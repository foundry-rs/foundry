//! Chisel is a fast, utilitarian, and verbose Solidity REPL.

// TODO(dani): remove async from all possible functions; mainly `execute` and bubble up
// TODO(dani): see if we can use &self in source with OnceCell

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate foundry_common;

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
