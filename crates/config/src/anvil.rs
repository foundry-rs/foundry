//! Configuration specific to the `anvil` command.

use crate::GasLimit;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Contains the config for `anvil` node settings.
///
/// This can be configured in `foundry.toml` under the `[anvil]` section:
///
/// ```toml
/// [anvil]
/// port = 8545
/// accounts = 10
/// balance = 10000
/// ```
///
/// Or under a profile-specific section:
///
/// ```toml
/// [profile.ci.anvil]
/// no_storage_caching = true
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnvilConfig {
    /// Port number to listen on.
    pub port: u16,

    /// The hosts the server will listen on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host: Vec<String>,

    /// The chain ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,

    /// The EVM hardfork to use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardfork: Option<String>,

    /// Number of dev accounts to generate and configure.
    pub accounts: u64,

    /// The balance of every dev account in Ether.
    pub balance: u64,

    /// BIP39 mnemonic phrase used for generating accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mnemonic: Option<String>,

    /// Sets the derivation path of the child key to be derived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,

    /// Block time in seconds for interval mining.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_time: Option<u64>,

    /// Disable auto and interval mining, and mine on demand instead.
    #[serde(default)]
    pub no_mining: bool,

    /// Enable mixed mining (interval + on-demand).
    #[serde(default)]
    pub mixed_mining: bool,

    /// How transactions are sorted in the mempool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,

    /// Slots in an epoch.
    pub slots_in_an_epoch: u64,

    /// The block gas limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<GasLimit>,

    /// Disable the `call.gas_limit <= block.gas_limit` constraint.
    #[serde(default)]
    pub disable_block_gas_limit: bool,

    /// Enable the transaction gas limit check (EIP-7825).
    #[serde(default)]
    pub enable_tx_gas_limit: bool,

    /// The gas price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<u128>,

    /// The base fee in a block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_base_fee_per_gas: Option<u64>,

    /// Disable the enforcement of a minimum suggested priority fee.
    #[serde(default)]
    pub disable_min_priority_fee: bool,

    /// Fetch state over a remote endpoint instead of starting from an empty state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_url: Option<String>,

    /// Fetch state from a specific block number over a remote endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<i128>,

    /// Specify chain id to skip fetching it from remote endpoint (offline mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_chain_id: Option<u64>,

    /// Headers to use for the rpc client.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fork_headers: Vec<String>,

    /// Timeout in ms for requests sent to remote JSON-RPC server in forking mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_request_timeout: Option<u64>,

    /// Number of retry requests for spurious networks (timed out requests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_request_retries: Option<u32>,

    /// Initial retry backoff on encountering errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_retry_backoff: Option<u64>,

    /// Sets the number of assumed available compute units per second for the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    #[serde(default)]
    pub no_rate_limit: bool,

    /// Explicitly disables the use of RPC caching.
    #[serde(default)]
    pub no_storage_caching: bool,

    /// EIP-170: Contract code size limit in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_size_limit: Option<usize>,

    /// Disable EIP-170: Contract code size limit.
    #[serde(default)]
    pub disable_code_size_limit: bool,

    /// The memory limit per EVM execution in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<u64>,

    /// Maximum number of transactions in a block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_transactions: Option<usize>,

    /// Enable steps tracing used for debug calls returning geth-style traces.
    #[serde(default)]
    pub steps_tracing: bool,

    /// Disable printing of `console.log` invocations to stdout.
    #[serde(default)]
    pub disable_console_log: bool,

    /// Enable printing of traces for executed transactions.
    #[serde(default)]
    pub print_traces: bool,

    /// Enables automatic impersonation on startup.
    #[serde(default)]
    pub auto_impersonate: bool,

    /// Disable the default create2 deployer.
    #[serde(default)]
    pub disable_default_create2_deployer: bool,

    /// Disable pool balance checks.
    #[serde(default)]
    pub disable_pool_balance_checks: bool,

    /// Don't keep full chain history. If a number is specified, at most this number of states is
    /// kept in memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_history: Option<Option<usize>>,

    /// Max number of states to persist on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_persisted_states: Option<usize>,

    /// Number of blocks with transactions to keep in memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_block_keeper: Option<usize>,

    /// Path to the cache directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_path: Option<PathBuf>,

    /// Launch an ipc server at the given path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipc: Option<String>,
}

impl Default for AnvilConfig {
    fn default() -> Self {
        Self {
            port: 8545,
            host: vec![],
            chain_id: None,
            hardfork: None,
            accounts: 10,
            balance: 10000,
            mnemonic: None,
            derivation_path: None,
            block_time: None,
            no_mining: false,
            mixed_mining: false,
            order: None,
            slots_in_an_epoch: 32,
            gas_limit: None,
            disable_block_gas_limit: false,
            enable_tx_gas_limit: false,
            gas_price: None,
            block_base_fee_per_gas: None,
            disable_min_priority_fee: false,
            fork_url: None,
            fork_block_number: None,
            fork_chain_id: None,
            fork_headers: vec![],
            fork_request_timeout: None,
            fork_request_retries: None,
            fork_retry_backoff: None,
            compute_units_per_second: None,
            no_rate_limit: false,
            no_storage_caching: false,
            code_size_limit: None,
            disable_code_size_limit: false,
            memory_limit: None,
            max_transactions: None,
            steps_tracing: false,
            disable_console_log: false,
            print_traces: false,
            auto_impersonate: false,
            disable_default_create2_deployer: false,
            disable_pool_balance_checks: false,
            prune_history: None,
            max_persisted_states: None,
            transaction_block_keeper: None,
            cache_path: None,
            ipc: None,
        }
    }
}
