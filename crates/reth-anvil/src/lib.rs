//! Reth-anvil is a local Ethereum development node built on the reth SDK.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod api;
pub mod args;
mod block_env;
pub mod cmd;
mod config;
mod console;
mod engine;
mod eth_api;
mod evm;
mod fork;
mod impersonation;
mod launcher;
mod logging;
mod miner;
mod mining;
mod network;
mod node;
pub mod opts;
mod pool;
mod provider;
mod server;
mod signer;
mod snapshot;
mod state;
mod state_dump;
mod state_provider;
mod time;
mod types;

pub use api::{
    AnvilApiServer, AnvilRpc, CLIENT_VERSION, EthExtApiServer, EvmApiServer, PersonalApiServer,
    Web3ExtApiServer,
};
pub use config::{
    AccountGenerator, CHAIN_ID, DEFAULT_GAS_LIMIT, DEFAULT_IPC_ENDPOINT, DEFAULT_MNEMONIC,
    DEFAULT_SLOTS_IN_AN_EPOCH, INITIAL_BASE_FEE, NODE_PORT, NodeConfig,
};
pub use eth_api::EthApi;
pub use fork::{ForkBackend, ForkSettings, LocalWrites};
pub use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
pub use node::{NodeHandle, spawn, try_spawn};
pub use state_dump::{SerializableAccountRecord, SerializableState, StateFile};
pub use types::{ForkChoice, ForkUrl, ReorgOptions, TransactionData, TransactionOrder};

pub use alloy_rpc_types::anvil::{Forking, Metadata, NodeInfo};

/// The in-process API under anvil's module path.
pub mod eth {
    pub use crate::eth_api::EthApi;
}
