//! Chisel is a fast, utilitarian, and verbose Solidity REPL.

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

pub mod session_source;

mod solidity_helper;
pub use solidity_helper::SolidityHelper;

pub mod prelude {
    pub use crate::{
        cmd::*, dispatcher::*, runner::*, session::*, session_source::*, solidity_helper::*,
    };
}
