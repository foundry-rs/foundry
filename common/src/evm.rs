//! cli arguments for configuring the evm settings
use clap::Parser;
use ethers_core::types::{Address, U256};
use foundry_config::{
    figment::{
        self,
        error::Kind::InvalidType,
        value::{Dict, Map, Value},
        Metadata, Profile, Provider,
    },
    Config,
};
use serde::Serialize;

/// `EvmArgs` and `EnvArgs` take the highest precedence in the Config/Figment hierarchy.
/// All vars are opt-in, their default values are expected to be set by the
/// [`foundry_config::Config`], and are always present ([`foundry_config::Config::default`])
///
/// Both have corresponding types in the `evm_adapters` crate which have mandatory fields.
/// The expected workflow is
///   1. load the [`foundry_config::Config`]
///   2. merge with `EvmArgs` into a `figment::Figment`
///   3. extract `evm_adapters::Opts` from the merged `Figment`
///
/// # Example
///
/// ```ignore
/// use foundry_config::Config;
/// use forge::executor::opts::EvmOpts;
/// use foundry_common::evm::EvmArgs;
/// # fn t(args: EvmArgs) {
/// let figment = Config::figment_with_root(".").merge(args);
/// let opts = figment.extract::<EvmOpts>().unwrap();
/// # }
/// ```
#[derive(Debug, Clone, Parser, Serialize)]
pub struct EvmArgs {
    /// Fetch state over a remote endpoint instead of starting from an empty state.
    ///
    /// If you want to fetch state from a specific block number, see --fork-block-number.
    #[clap(long, short, alias = "rpc-url", value_name = "URL")]
    #[serde(rename = "eth_rpc_url", skip_serializing_if = "Option::is_none")]
    pub fork_url: Option<String>,

    /// Fetch state from a specific block number over a remote endpoint.
    ///
    /// See --fork-url.
    #[clap(long, requires = "fork-url", value_name = "BLOCK")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_block_number: Option<u64>,

    /// Explicitly disables the use of RPC caching.
    ///
    /// All storage slots are read entirely from the endpoint.
    ///
    /// This flag overrides the project's configuration file.
    ///
    /// See --fork-url.
    #[clap(long, requires = "fork-url")]
    #[serde(skip)]
    pub no_storage_caching: bool,

    /// The initial balance of deployed test contracts.
    #[clap(long, value_name = "BALANCE")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_balance: Option<U256>,

    /// The address which will be executing tests.
    #[clap(long, value_name = "ADDRESS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Address>,

    /// Enable the FFI cheatcode.
    #[clap(help = "Enables the FFI cheatcode.", long)]
    #[serde(skip)]
    pub ffi: bool,

    /// Verbosity of the EVM.
    ///
    /// Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
    ///
    /// Verbosity levels:
    /// - 2: Print logs for all tests
    /// - 3: Print execution traces for failing tests
    /// - 4: Print execution traces for all tests, and setup traces for failing tests
    /// - 5: Print execution and setup traces for all tests
    #[clap(long, short, parse(from_occurrences), verbatim_doc_comment)]
    #[serde(skip)]
    pub verbosity: u8,

    /// All ethereum environment related arguments
    #[clap(flatten)]
    #[serde(flatten)]
    pub env: EnvArgs,
}

// Make this set of options a `figment::Provider` so that it can be merged into the `Config`
impl Provider for EvmArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Evm Opts Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let value = Value::serialize(self)?;
        let error = InvalidType(value.to_actual(), "map".into());
        let mut dict = value.into_dict().ok_or(error)?;

        if self.verbosity > 0 {
            // need to merge that manually otherwise `from_occurrences` does not work
            dict.insert("verbosity".to_string(), self.verbosity.into());
        }

        if self.ffi {
            dict.insert("ffi".to_string(), self.ffi.into());
        }

        if self.no_storage_caching {
            dict.insert("no_storage_caching".to_string(), self.no_storage_caching.into());
        }

        if let Some(fork_url) = &self.fork_url {
            dict.insert("eth_rpc_url".to_string(), fork_url.clone().into());
        }

        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// Configures the executor environment during tests.
#[derive(Debug, Clone, Default, Parser, Serialize)]
#[clap(next_help_heading = "EXECUTOR ENVIRONMENT CONFIG")]
pub struct EnvArgs {
    /// The block gas limit.
    #[clap(long, value_name = "GAS_LIMIT")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<u64>,

    /// The chain ID.
    #[clap(long, value_name = "CHAIN_ID")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,

    /// The gas price.
    #[clap(long, value_name = "GAS_PRICE")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<u64>,

    /// The base fee in a block.
    #[clap(long, value_name = "FEE")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_base_fee_per_gas: Option<u64>,

    /// The transaction origin.
    #[clap(long, value_name = "ADDRESS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_origin: Option<Address>,

    /// The coinbase of the block.
    #[clap(long, value_name = "ADDRESS")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_coinbase: Option<Address>,

    /// The timestamp of the block.
    #[clap(long, value_name = "TIMESTAMP")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_timestamp: Option<u64>,

    /// The block number.
    #[clap(long, value_name = "BLOCK")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,

    /// The block difficulty.
    #[clap(long, value_name = "DIFFICULTY")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_difficulty: Option<u64>,

    /// The block gas limit.
    #[clap(long, value_name = "GAS_LIMIT")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_gas_limit: Option<u64>,
}
