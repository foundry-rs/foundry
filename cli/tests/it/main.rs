use std::sync::atomic::{AtomicU16, Ordering};
#[cfg(not(feature = "external-integration-tests"))]
mod cast;
#[cfg(not(feature = "external-integration-tests"))]
mod cmd;
#[cfg(not(feature = "external-integration-tests"))]
mod config;
#[cfg(not(feature = "external-integration-tests"))]
mod create;
#[cfg(not(feature = "external-integration-tests"))]
mod script;
#[cfg(not(feature = "external-integration-tests"))]
mod test_cmd;
#[cfg(not(feature = "external-integration-tests"))]
mod utils;
#[cfg(not(feature = "external-integration-tests"))]
mod verify;

// import forge utils as mod
#[allow(unused)]
#[path = "../../src/utils.rs"]
pub(crate) mod forge_utils;

#[cfg(feature = "external-integration-tests")]
mod integration;

// keeps track of ports that can be used
pub static NEXT_PORT: AtomicU16 = AtomicU16::new(8546);

pub fn next_port() -> u16 {
    NEXT_PORT.fetch_add(1, Ordering::SeqCst)
}

fn main() {}
