use std::{
    net::TcpStream,
    sync::atomic::{AtomicU16, Ordering},
};
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

/// Returns the next free port to use
pub fn next_port() -> u16 {
    loop {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        // while simply incrementing ports is fine for a single test process, there might be
        // multiple concurrent anvil related test process, so we check if the port is already in use
        if TcpStream::connect(("0.0.0.0", port)).is_err() {
            return port
        }
    }
}

fn main() {}
