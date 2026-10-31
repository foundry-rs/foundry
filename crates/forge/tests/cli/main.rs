#[macro_use]
extern crate foundry_test_utils;

pub mod constants;
pub mod utils;

#[path = "../support/lsp_client.rs"]
mod lsp_client;

mod backtrace;
mod bind;
mod bind_json;
mod build;
mod cache;
mod cmd;
mod compiler;
mod config;
mod context;
mod coverage;
mod create;
mod debug;
mod doc;
mod eip712;
mod exploit;
mod failure_assertions;
mod http;
mod inline_config;
mod install;
mod lsp;

mod json;
mod lint;
mod multi_script;
mod precompiles;
mod properties;
mod script;
mod selectors;
mod soldeer;
mod storage_layout;
mod svm;
mod test_cmd;
mod verify;
mod verify_bytecode;
mod version;
mod watch;

mod ext_integration;
mod fmt;
mod fmt_integration;
mod test_optimizer;

#[cfg(unix)]
mod tempo_proofs;
