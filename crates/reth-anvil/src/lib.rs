//! Reth-anvil is a local Ethereum development node built on the reth SDK.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// Anvil's Optimism and Base tests use these until the networks run.
#[cfg(feature = "base")]
use base_common_consensus as _;
#[cfg(feature = "base")]
use base_common_precompiles as _;
#[cfg(any(feature = "optimism", feature = "base"))]
use op_alloy_consensus as _;
#[cfg(feature = "optimism")]
use op_alloy_rpc_types as _;

pub mod api;
mod beacon;
pub mod block_env;
pub mod config;
pub mod console;
mod debug;
mod engine;
mod eth_api;
pub mod evm;
pub mod fork;
mod history;
pub mod impersonation;
mod launcher;
pub mod logging;
mod miner;
mod mining;
pub mod network;
mod node;
mod otterscan;
pub mod pending;
pub mod pool;
pub mod provider;
mod server;
mod signer;
mod simulate;
mod snapshot;
pub mod state;
pub mod state_dump;
mod state_provider;
pub mod time;
pub mod txpool;
mod types;

pub use api::{
    AnvilApiServer, AnvilRpc, CLIENT_VERSION, CallBatch, EthExtApiServer, EvmApiServer,
    PersonalApiServer, Web3ExtApiServer,
};
pub use config::{
    AccountGenerator, CHAIN_ID, DEFAULT_GAS_LIMIT, DEFAULT_IPC_ENDPOINT, DEFAULT_MNEMONIC,
    DEFAULT_SLOTS_IN_AN_EPOCH, ForkSource, INITIAL_BASE_FEE, NODE_PORT, NodeConfig,
};
pub use eth_api::EthApi;
pub use evm::AnvilExecutorBuilder;
pub use fork::{ForkBackend, ForkNetwork, ForkSettings, LocalWrites};
pub use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
pub use network::{AnvilComponents, AnvilNetwork, AnvilRpcOf};
pub use node::{NodeHandle, launch, spawn, try_spawn};
pub use state_dump::{SerializableAccountRecord, SerializableState, StateFile};
pub use txpool::TxPoolKey;
pub use types::{ForkChoice, ForkUrl, ReorgOptions, TransactionData, TransactionOrder};

pub use alloy_rpc_types::anvil::{Forking, Metadata, NodeInfo};

/// The in-process API under anvil's module path.
pub mod eth {
    pub use crate::eth_api::EthApi;
}
