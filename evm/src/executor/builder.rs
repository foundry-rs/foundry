use ethers::prelude::Provider;
use revm::{
    db::{DatabaseRef, EmptyDB},
    Env, SpecId,
};
use std::{path::PathBuf, sync::Arc};

use super::{
    fork::{SharedBackend, SharedMemCache},
    inspector::InspectorStackConfig,
    Executor,
};

use crate::storage::StorageMap;
use ethers::types::{H160, H256, U256};

use parking_lot::lock_api::RwLock;
use revm::AccountInfo;

#[derive(Default, Debug)]
pub struct ExecutorBuilder {
    /// The execution environment configuration.
    env: Env,
    /// The configuration used to build an [InspectorStack].
    inspector_config: InspectorStackConfig,
    fork: Option<Fork>,
}

#[derive(Clone, Debug)]
pub struct Fork {
    /// Where to read the cached storage from
    pub cache_storage: Option<PathBuf>,
    /// The URL to a node for fetching remote state
    pub url: String,
    /// The block to fork against
    pub pin_block: Option<u64>,
}

impl Fork {
    /// Initialises the Storage Backend
    ///
    /// If configured, then this will initialise the backend with the storage cahce
    fn into_backend(self) -> SharedBackend {
        let Fork { cache_storage, url, pin_block } = self;
        let provider = Provider::try_from(url).expect("Failed to establish provider");

        let mut storage_map = if let Some(cached_storage) = cache_storage {
            StorageMap::read(cached_storage)
        } else {
            StorageMap::transient()
        };

        SharedBackend::new(
            provider,
            SharedMemCache {
                storage: Arc::new(RwLock::new(storage_map.take_storage())),
                ..Default::default()
            },
            pin_block.map(Into::into),
            storage_map,
        )
    }
}

pub enum Backend {
    Simple(EmptyDB),
    Forked(SharedBackend),
}

impl Backend {
    /// Instantiates a new backend union based on whether there was or not a fork url specified
    fn new(fork: Option<Fork>) -> Self {
        if let Some(fork) = fork {
            Backend::Forked(fork.into_backend())
        } else {
            Backend::Simple(EmptyDB())
        }
    }
}

impl DatabaseRef for Backend {
    fn basic(&self, address: H160) -> AccountInfo {
        match self {
            Backend::Simple(inner) => inner.basic(address),
            Backend::Forked(inner) => inner.basic(address),
        }
    }

    fn code_by_hash(&self, address: H256) -> bytes::Bytes {
        match self {
            Backend::Simple(inner) => inner.code_by_hash(address),
            Backend::Forked(inner) => inner.code_by_hash(address),
        }
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        match self {
            Backend::Simple(inner) => inner.storage(address, index),
            Backend::Forked(inner) => inner.storage(address, index),
        }
    }

    fn block_hash(&self, number: U256) -> H256 {
        match self {
            Backend::Simple(inner) => inner.block_hash(number),
            Backend::Forked(inner) => inner.block_hash(number),
        }
    }
}

impl ExecutorBuilder {
    #[must_use]
    pub fn new() -> Self {
        Default::default()
    }

    /// Enables cheatcodes on the executor.
    #[must_use]
    pub fn with_cheatcodes(mut self, ffi: bool) -> Self {
        self.inspector_config.cheatcodes = true;
        self.inspector_config.ffi = ffi;
        self
    }

    /// Enables tracing
    #[must_use]
    pub fn with_tracing(mut self) -> Self {
        self.inspector_config.tracing = true;
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

    /// Configure the execution environment (gas limit, chain spec, ...)
    #[must_use]
    pub fn with_config(mut self, env: Env) -> Self {
        self.inspector_config.block = env.block.clone();
        self.env = env;
        self
    }

    /// Configure the executor's forking mode
    #[must_use]
    pub fn with_fork(mut self, fork: Option<Fork>) -> Self {
        self.fork = fork;
        self
    }

    /// Builds the executor as configured.
    pub fn build(self) -> Executor<Backend> {
        let db = Backend::new(self.fork);
        Executor::new(db, self.env, self.inspector_config)
    }
}
