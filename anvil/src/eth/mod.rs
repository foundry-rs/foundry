pub mod api;
mod otterscan;
pub use api::EthApi;

pub mod backend;

pub mod error;

pub mod fees;
pub(crate) mod macros;
pub mod miner;
pub mod pool;
pub mod sign;
pub mod util;
