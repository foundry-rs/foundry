use std::{
    net::TcpStream,
    sync::atomic::{AtomicU16, Ordering},
};

mod abi;
mod anvil;
mod anvil_api;
mod api;
mod fork;
mod ganache;
mod gas;
mod geth;
mod logs;
mod pubsub;
mod revert;
mod traces;
mod transaction;
mod txpool;
pub mod utils;
mod wsapi;

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

#[allow(unused)]
pub(crate) fn init_tracing() {
    let _ = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

fn main() {}
