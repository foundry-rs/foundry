#[macro_use]
extern crate foundry_test_utils;

pub mod constants;
pub mod utils;

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
mod geiger;
mod inline_config;
mod multi_script;
mod odyssey;
mod script;
mod soldeer;
mod svm;
mod test_cmd;
mod verify;
mod verify_bytecode;

mod ext_integration;
