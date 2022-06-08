use super::{
    fork::SharedBackend,
    inspector::{Cheatcodes, InspectorStackConfig},
    Executor,
};
use crate::executor::{
    backend::Backend,
    fork::{BlockchainDb, BlockchainDbMeta},
};
use ethers::{
    providers::{Http, Provider, RetryClient},
    types::U256,
};
use foundry_config::cache::StorageCachingConfig;
use revm::{Env, SpecId};
use std::{path::PathBuf, sync::Arc};

#[derive(Default, Debug)]
pub struct ExecutorBuilder {
    /// The execution environment configuration.
    env: Env,
    /// The configuration used to build an [InspectorStack].
    inspector_config: InspectorStackConfig,
    gas_limit: Option<U256>,
}

// === impl ExecutorBuilder ===

impl ExecutorBuilder {
    /// Enables cheatcodes on the executor.
    #[must_use]
    pub fn with_cheatcodes(mut self, ffi: bool, rpc_storage_caching: StorageCachingConfig) -> Self {
        self.inspector_config.cheatcodes = Some(Cheatcodes::new(
            ffi,
            self.env.block.clone(),
            self.env.tx.gas_price,
            rpc_storage_caching,
        ));
        self
    }

    /// Enables tracing
    #[must_use]
    pub fn with_tracing(self) -> Self {
        self.set_tracing(true)
    }

    /// Sets the tracing verbosity
    #[must_use]
    pub fn set_tracing(mut self, with_tracing: bool) -> Self {
        self.inspector_config.tracing = with_tracing;
        self
    }

    /// Enables the debugger
    #[must_use]
    pub fn with_debugger(mut self) -> Self {
        self.inspector_config.debugger = true;
        self
    }

    /// Sets the EVM spec to use
    #[must_use]
    pub fn with_spec(mut self, spec: SpecId) -> Self {
        self.env.cfg.spec_id = spec;
        self
    }

    /// Sets the executor gas limit.
    ///
    /// See [Executor::gas_limit] for more info on why you might want to set this.
    #[must_use]
    pub fn with_gas_limit(mut self, gas_limit: U256) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Configure the execution environment (gas limit, chain spec, ...)
    #[must_use]
    pub fn with_config(mut self, env: Env) -> Self {
        self.inspector_config.block = env.block.clone();
        self.inspector_config.gas_price = env.tx.gas_price;
        self.env = env;
        self
    }

    /// Builds the executor as configured.
    pub fn build(self, db: Backend) -> Executor {
        let gas_limit = self.gas_limit.unwrap_or(self.env.block.gas_limit);
        Executor::new(db, self.env, self.inspector_config, gas_limit)
    }
}

/// Represents a _fork_ of a live chain whose data is available only via the `url` endpoint.
///
/// *Note:* this type intentionally does not implement `Clone` to prevent [Fork::spawn_backend()]
/// from being called multiple times.
#[derive(Debug)]
pub struct Fork {
    /// Where to read the cached storage from
    pub cache_path: Option<PathBuf>,
    /// The URL to a node for fetching remote state
    pub url: String,
    /// The block to fork against
    pub pin_block: Option<u64>,
    /// chain id retrieved from the endpoint
    pub chain_id: u64,
    /// The initial retry backoff
    pub initial_backoff: u64,
}

impl Fork {
    /// Initialises and spawns the Storage Backend, the [revm::Database]
    ///
    /// If configured, then this will initialise the backend with the storage cache.
    ///
    /// The `SharedBackend` returned is connected to a background thread that communicates with the
    /// endpoint via channels and is intended to be cloned when multiple [revm::Database] are
    /// required. See also [crate::executor::fork::SharedBackend]
    pub async fn spawn_backend(self, env: &Env) -> SharedBackend {
        let Fork { cache_path, url, pin_block, chain_id, initial_backoff } = self;

        let provider = Arc::new(
            Provider::<RetryClient<Http>>::new_client(url.clone().as_str(), 10, initial_backoff)
                .expect("Failed to establish provider"),
        );

        let mut meta = BlockchainDbMeta::new(env.clone(), url);

        // update the meta to match the forked config
        meta.cfg_env.chain_id = chain_id.into();
        if let Some(pin) = pin_block {
            meta.block_env.number = pin.into();
        }

        let db = BlockchainDb::new(meta, cache_path);

        SharedBackend::spawn_backend(provider, db, pin_block.map(Into::into)).await
    }
}
