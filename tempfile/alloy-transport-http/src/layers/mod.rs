//! tower http like layer implementations that work over the http::Request type.
#![cfg(all(not(target_arch = "wasm32"), feature = "hyper"))]

#[cfg(feature = "jwt-auth")]
mod auth;
#[cfg(feature = "jwt-auth")]
pub use auth::{AuthLayer, AuthService};
