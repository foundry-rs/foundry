mod backend;

use super::opts::EvmOpts;
pub use backend::{BackendHandler, SharedBackend};

use revm::primitives::Env;

mod init;
pub use init::environment;

mod cache;
pub use cache::{BlockchainDb, BlockchainDbMeta, JsonBlockCacheDB, MemDb};

pub mod database;

mod multi;
pub use multi::{ForkId, MultiFork, MultiForkHandler};

/// Represents a _fork_ of a remote chain whose data is available only via the `url` endpoint.
#[derive(Debug, Clone)]
pub struct CreateFork {
    /// Whether to enable rpc storage caching for this fork
    pub enable_caching: bool,
    /// The URL to a node for fetching remote state
    pub url: String,
    /// The env to create this fork, main purpose is to provide some metadata for the fork
    pub env: Env,
    /// All env settings as configured by the user
    pub evm_opts: EvmOpts,
}
