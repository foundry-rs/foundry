//! Useful layer implementations for the provider. Currently this
//! module contains the `AnvilLayer`, `AnvilProvider` and `ChainLayer`
//! types.

#[cfg(any(test, feature = "anvil-node"))]
mod anvil;
#[cfg(any(test, feature = "anvil-node"))]
pub use anvil::{AnvilLayer, AnvilProvider};

mod chain;
pub use chain::ChainLayer;

#[cfg(not(target_arch = "wasm32"))]
mod cache;
#[cfg(not(target_arch = "wasm32"))]
pub use cache::{CacheLayer, CacheProvider, SharedCache};
