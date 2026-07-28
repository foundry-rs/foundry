//! MPP (Machine Payments Protocol) support for 402-gated RPC endpoints.
//!
//! - [`transport`]: HTTP transport that handles 402 challenges automatically.
//! - [`ws`]: WebSocket transport that performs the same Charge handshake.

pub mod transport;
pub mod ws;
