extern crate core;

// Macros useful for testing.
mod macros;

// Utilities for making it easier to handle tests.
pub mod util;
pub use util::{TestCommand, TestProject};

pub use ethers_solc;

pub mod rpc;
pub use rpc::next_http_rpc_endpoint;
