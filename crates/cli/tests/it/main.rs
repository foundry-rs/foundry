#[cfg(not(feature = "external-integration-tests"))]
mod cache;
#[cfg(not(feature = "external-integration-tests"))]
mod cast;
#[cfg(not(feature = "external-integration-tests"))]
mod cmd;
#[cfg(not(feature = "external-integration-tests"))]
mod config;
#[cfg(not(feature = "external-integration-tests"))]
mod create;
#[cfg(not(feature = "external-integration-tests"))]
mod doc;
#[cfg(not(feature = "external-integration-tests"))]
mod multi_script;
#[cfg(not(feature = "external-integration-tests"))]
mod script;
mod svm;
#[cfg(not(feature = "external-integration-tests"))]
mod test_cmd;
#[cfg(not(feature = "external-integration-tests"))]
mod utils;
#[cfg(not(feature = "external-integration-tests"))]
mod verify;

// import forge utils as mod
#[allow(unused)]
#[path = "../../src/utils/mod.rs"]
pub(crate) mod forge_utils;

#[cfg(feature = "external-integration-tests")]
mod integration;

pub mod constants;

fn main() {}
