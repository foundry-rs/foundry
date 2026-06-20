//! MPP (Machine Payments Protocol) support for 402-gated RPC endpoints.
//!
//! - [`keys`]: Auto-discovery of signing keys from the Tempo wallet.
//! - [`transport`]: HTTP transport that handles 402 challenges automatically.

pub mod keys;
pub mod persist;
pub mod session;
pub mod transport;
pub mod ws;
