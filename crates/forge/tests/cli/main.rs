#[macro_use]
extern crate foundry_test_utils;

pub mod constants;
pub mod utils;

mod cache;
mod cmd;
mod config;
mod coverage;
mod create;
mod doc;
mod multi_script;
mod script;
mod svm;
mod test_cmd;
mod verify;

mod ext_integration;

#[cfg(feature = "heavy-integration-tests")]
mod heavy_integration;
