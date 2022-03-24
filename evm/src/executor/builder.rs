use ethers::prelude::Provider;
use revm::{
    db::{DatabaseRef, EmptyDB},
    Env, SpecId,
};
use std::path::PathBuf;

use super::{fork::SharedBackend, inspector::InspectorStackConfig, Executor};

use ethers::types::{H160, H256, U256};

use crate::executor::fork::{BlockchainDb, BlockchainDbMeta};

use revm::AccountInfo;
use url::Url;

#[derive(Default, Debug)]
pub struct ExecutorBuilder {
    /// The execution environment configuration.
    env: Env,
    /// The configuration used to build an [InspectorStack].
    inspector_config: InspectorStackConfig,
    fork: Option<Fork>,
    gas_limit: Option<U256>,
}

#[derive(Clone, Debug)]
pub struct Fork {
    /// Where to read the cached storage from
    pub cache_path: Option<PathBuf>,
    /// The URL to a node for fetching remote state
    pub url: String,
    /// The block to fork against
    pub pin_block: Option<u64>,
    /// chain id retrieved from the endpoint
    pub chain_id: u64,
}

impl Fork {
    /// Initialises the Storage Backend
    ///
    /// If configured, then this will initialise the backend with the storage cache
    pub fn into_backend(self, env: &Env) -> SharedBackend {
        let Fork { cache_path, url, pin_block, chain_id } = self;

        let host = Url::parse(&url)
            .ok()
            .and_then(|url| url.host().map(|host| host.to_string()))
            .unwrap_or_else(|| url.clone());

        let provider = Provider::try_from(url).expect("Failed to establish provider");

        let mut meta =
            BlockchainDbMeta { cfg_env: env.cfg.clone(), block_env: env.block.clone(), host };

        // update the meta to match the forked config
        meta.cfg_env.chain_id = chain_id.into();
        if let Some(pin) = pin_block {
            meta.block_env.number = pin.into();
        }

        let db = BlockchainDb::new(meta, cache_path);

        SharedBackend::new(provider, db, pin_block.map(Into::into))
    }
}

pub enum Backend {
    Simple(EmptyDB),
    Forked(SharedBackend),
}

impl Backend {
    /// Instantiates a new backend union based on whether there was or not a fork url specified
    fn new(fork: Option<Fork>, env: &Env) -> Self {
        if let Some(fork) = fork {
            Backend::Forked(fork.into_backend(env))
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
        let db = Backend::new(self.fork, &self.env);
        let gas_limit = self.gas_limit.unwrap_or(self.env.block.gas_limit);
        Executor::new(db, self.env, self.inspector_config, gas_limit)
    }
}
