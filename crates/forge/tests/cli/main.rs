#[cfg(not(feature = "external-integration-tests"))]
mod cache;
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

#[cfg(feature = "external-integration-tests")]
mod integration;

pub mod constants;
