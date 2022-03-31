#[cfg(not(feature = "external-integration-tests"))]
mod cast;
#[cfg(not(feature = "external-integration-tests"))]
mod cmd;
#[cfg(not(feature = "external-integration-tests"))]
mod config;
#[cfg(not(feature = "external-integration-tests"))]
mod debug;
#[cfg(not(feature = "external-integration-tests"))]
mod test;

// import forge utils as mod
#[allow(unused)]
#[path = "../../src/utils.rs"]
mod forge_utils;

#[cfg(feature = "external-integration-tests")]
mod integration;

fn main() {}
