use crate::{
    FeeManager, PrecompileFactory,
    eth::{
        backend::{
            db::{Db, SerializableState},
            fork::{
                ClientFork, ClientForkConfig, ForkEndpointIdentity, ensure_fork_network_supported,
            },
            genesis::GenesisConfig,
            mem::fork_db::ForkedDatabase,
            time::duration_since_unix_epoch,
        },
        fees::{INITIAL_BASE_FEE, INITIAL_GAS_PRICE},
        pool::transactions::TransactionOrder,
    },
    mem::{self, in_memory_db::StateRootDb},
};
use alloy_chains::{Chain, NamedChain};
use alloy_consensus::BlockHeader;
use alloy_eips::{eip1559::BaseFeeParams, eip7840::BlobParams};
use alloy_evm::EvmEnv;
use alloy_genesis::Genesis;
use alloy_network::{AnyNetwork, AnyRpcBlock, BlockResponse, TransactionResponse};
use alloy_primitives::{
    Address, B256, BlockNumber, TxHash, U256, hex, keccak256, map::HashMap, utils::Unit,
};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockNumberOrTag,
    anvil::{Metadata, NodeInfo},
};
use alloy_signer::Signer;
use alloy_signer_local::{
    MnemonicBuilder, PrivateKeySigner,
    coins_bip39::{English, Mnemonic},
};
use alloy_transport::TransportError;
use anvil_server::ServerConfig;
use eyre::{Context, Result};
use foundry_common::{
    ALCHEMY_FREE_TIER_CUPS, NON_ARCHIVE_NODE_WARNING, REQUEST_TIMEOUT,
    provider::{ProviderBuilder, RetryProvider, is_rpc_method_not_found, redact_url},
};
use foundry_config::Config;
use foundry_evm::{
    backend::{BlockchainDb, BlockchainDbMeta, ForkBlock, SharedBackend},
    constants::DEFAULT_CREATE2_DEPLOYER,
    hardfork::FoundryHardfork,
    utils::{apply_chain_and_block_specific_env_changes_for_chain, block_env_from_header},
};
use parking_lot::RwLock;
use rand_08::thread_rng;
use revm::{
    context::{BlockEnv, CfgEnv},
    context_interface::block::BlobExcessGasAndPrice,
    primitives::hardfork::SpecId,
};
use serde_json::{Value, json};
use std::{
    fmt::Write as FmtWrite,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tempo_hardfork::{
    TempoHardfork,
    constants::gas::{TEMPO_T0_BASE_FEE, TEMPO_T1_BASE_FEE},
};
use tokio::sync::RwLock as TokioRwLock;
use yansi::Paint;

pub use foundry_common::version::SHORT_VERSION as VERSION_MESSAGE;
use foundry_evm::{
    traces::{CallTraceDecoderBuilder, identifier::SignaturesIdentifier},
    utils::{get_blob_params, get_blob_params_by_hardfork},
};
use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
use tempo_precompiles::TIP_FEE_MANAGER_ADDRESS;

/// Default port the rpc will open
pub const NODE_PORT: u16 = 8545;
/// Default chain id of the node
pub const CHAIN_ID: u64 = 31337;
/// The default gas limit for all transactions
pub const DEFAULT_GAS_LIMIT: u64 = 30_000_000;
/// The default number of slots in an epoch used for safe/finalized block tags.
pub const DEFAULT_SLOTS_IN_AN_EPOCH: u64 = 32;
/// Default mnemonic for dev accounts
pub const DEFAULT_MNEMONIC: &str = "test test test test test test test test test test test junk";

#[derive(Clone, Copy, Debug)]
struct ForkOverrides {
    gas_limit: Option<u64>,
    gas_price: Option<u128>,
    base_fee: Option<u64>,
}

struct StableForkSnapshot {
    endpoint_identity: ForkEndpointIdentity,
    block_number: u64,
    transaction_replay: Option<ForkTransactionReplay>,
    block: Option<AnyRpcBlock>,
    gas_price: u128,
}

/// Best-effort Anvil detection that becomes strict after positive identification.
///
/// Before the first successful `anvil_nodeInfo` response, any probe failure means that the
/// optional capability is unavailable. Mandatory standard RPC reads still expose endpoint-wide
/// failures. Once a response or cached endpoint identity identifies Anvil, every later probe
/// failure is returned so it cannot hide an endpoint reset or execution-profile change.
#[derive(Clone, Copy, Debug, Default)]
struct AnvilNodeInfoProbe {
    identified: bool,
}

impl AnvilNodeInfoProbe {
    const fn new(identified: bool) -> Self {
        Self { identified }
    }

    async fn request(&mut self, provider: &RetryProvider) -> Result<Option<NodeInfo>> {
        match provider.raw_request::<_, NodeInfo>("anvil_nodeInfo".into(), ()).await {
            Ok(node_info) => {
                self.identified = true;
                Ok(Some(node_info))
            }
            Err(_) if !self.identified => Ok(None),
            Err(error) => {
                Err(error).wrap_err("failed to determine network family from fork endpoint")
            }
        }
    }
}

/// One-shot source data for a transaction-hash fork replay.
#[derive(Clone, Debug)]
pub(crate) struct ForkTransactionReplay {
    pub(crate) source_block: AnyRpcBlock,
    pub(crate) target_index: usize,
}

/// The default IPC endpoint
pub const DEFAULT_IPC_ENDPOINT: &str =
    if cfg!(unix) { "/tmp/anvil.ipc" } else { r"\\.\pipe\anvil.ipc" };

const BANNER: &str = r"
                             _   _
                            (_) | |
      __ _   _ __   __   __  _  | |
     / _` | | '_ \  \ \ / / | | | |
    | (_| | | | | |  \ V /  | | | |
     \__,_| |_| |_|   \_/   |_| |_|
";

fn fork_source_id(urls: &[String], headers: &[String]) -> B256 {
    let mut encoded = Vec::new();
    for parts in [urls, headers] {
        encoded.extend_from_slice(&(parts.len() as u64).to_be_bytes());
        for part in parts {
            encoded.extend_from_slice(&(part.len() as u64).to_be_bytes());
            encoded.extend_from_slice(part.as_bytes());
        }
    }
    keccak256(encoded)
}

/// Configurations of the EVM node
#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// Chain ID of the EVM chain
    pub chain_id: Option<u64>,
    /// Default gas limit for all txs
    pub gas_limit: Option<u64>,
    /// If set to `true`, disables the block gas limit
    pub disable_block_gas_limit: bool,
    /// If set to `true`, enables the tx gas limit as imposed by Osaka (EIP-7825)
    pub enable_tx_gas_limit: bool,
    /// Default gas price for all txs
    pub gas_price: Option<u128>,
    /// Default base fee
    pub base_fee: Option<u64>,
    /// If set to `true`, disables the enforcement of a minimum suggested priority fee
    pub disable_min_priority_fee: bool,
    /// Default blob excess gas and price
    pub blob_excess_gas_and_price: Option<BlobExcessGasAndPrice>,
    /// The hardfork to force, or `None` to infer it from chain activation data.
    pub hardfork: Option<FoundryHardfork>,
    /// Signer accounts that will be initialised with `genesis_balance` in the genesis block
    pub genesis_accounts: Vec<PrivateKeySigner>,
    /// Native token balance of every genesis account in the genesis block
    pub genesis_balance: U256,
    /// Genesis block timestamp
    pub genesis_timestamp: Option<u64>,
    /// Genesis block number
    pub genesis_block_number: Option<u64>,
    /// Signer accounts that can sign messages/transactions from the EVM node
    pub signer_accounts: Vec<PrivateKeySigner>,
    /// Configured block time for the EVM chain. Use `None` for instant/auto mining.
    pub block_time: Option<Duration>,
    /// Disable auto and interval mining mode and use `MiningMode::None` instead.
    pub no_mining: bool,
    /// Enables auto and interval mining mode
    pub mixed_mining: bool,
    /// port to use for the server
    pub port: u16,
    /// maximum number of transactions in a block
    pub max_transactions: usize,
    /// Fork URLs for RPC calls. The first entry is the primary endpoint.
    /// When multiple URLs are provided, requests are distributed using
    /// round-robin load balancing with retry-based failover.
    pub fork_urls: Vec<String>,
    /// pins the block number or transaction hash for the state fork
    pub fork_choice: Option<ForkChoice>,
    /// headers to use with fork RPC endpoints
    pub fork_headers: Vec<String>,
    /// specifies chain id for cache to skip fetching from remote in offline-start mode
    pub fork_chain_id: Option<U256>,
    /// Chain ID discovered from the active fork source.
    pub fork_source_chain_id: Option<u64>,
    /// Chain ID exposed by the active fork endpoint.
    pub fork_execution_chain_id: Option<u64>,
    /// Whether the active fork endpoint has positively identified itself as Anvil.
    pub(crate) fork_endpoint_is_anvil: bool,
    /// Network family most recently inferred from a fork endpoint.
    inferred_fork_network: Option<NetworkVariant>,
    /// Network configuration replaced by chain-ID inference, if any.
    chain_id_network_base: Option<NetworkConfigs>,
    /// User-provided gas settings captured before fork-derived values are materialized.
    fork_overrides: Option<ForkOverrides>,
    /// The generator used to generate the dev accounts
    pub account_generator: Option<AccountGenerator>,
    /// whether to enable tracing
    pub enable_tracing: bool,
    /// Explicitly disables the use of RPC caching.
    pub no_storage_caching: bool,
    /// How to configure the server
    pub server_config: ServerConfig,
    /// The host the server will listen on
    pub host: Vec<IpAddr>,
    /// How transactions are sorted in the mempool
    pub transaction_order: TransactionOrder,
    /// Filename to write anvil output as json
    pub config_out: Option<PathBuf>,
    /// The genesis to use to initialize the node
    pub genesis: Option<Genesis>,
    /// Timeout in for requests sent to remote JSON-RPC server in forking mode
    pub fork_request_timeout: Duration,
    /// Number of request retries for spurious networks
    pub fork_request_retries: u32,
    /// The initial retry backoff
    pub fork_retry_backoff: Duration,
    /// available CUPS
    pub compute_units_per_second: u64,
    /// The ipc path
    pub ipc_path: Option<Option<String>>,
    /// Enable transaction/call steps tracing for debug calls returning geth-style traces
    pub enable_steps_tracing: bool,
    /// Enable printing of `console.log` invocations.
    pub print_logs: bool,
    /// Enable printing of traces.
    pub print_traces: bool,
    /// Enable auto impersonation of accounts on startup
    pub enable_auto_impersonate: bool,
    /// Configure the code size limit
    pub code_size_limit: Option<usize>,
    /// Configures how to remove historic state.
    ///
    /// If set to `Some(num)` keep latest num state in memory only.
    pub prune_history: PruneStateHistoryConfig,
    /// Max number of states cached on disk.
    pub max_persisted_states: Option<usize>,
    /// The initial state to apply and consume during startup.
    pub init_state: Option<SerializableState>,
    /// max number of blocks with transactions in memory
    pub transaction_block_keeper: Option<usize>,
    /// Disable the default CREATE2 deployer
    pub disable_default_create2_deployer: bool,
    /// Disable pool balance checks
    pub disable_pool_balance_checks: bool,
    /// Slots in an epoch
    pub slots_in_an_epoch: u64,
    /// The memory limit per EVM execution in bytes.
    pub memory_limit: Option<u64>,
    /// Factory used by `anvil` to extend the EVM's precompiles.
    pub precompile_factory: Option<Arc<dyn PrecompileFactory>>,
    /// Networks to enable features for.
    pub networks: NetworkConfigs,
    /// Overrides the Base activation-registry administrator.
    #[cfg(feature = "base")]
    pub base_activation_admin: Option<Address>,
    /// The account used to sponsor Tempo fee-payer requests.
    ///
    /// Must be an unlocked signer account. Defaults to the last dev account on Tempo networks.
    pub tempo_fee_payer: Option<Address>,
    /// Do not print log messages.
    pub silent: bool,
    /// The path where persisted states are cached (used with `max_persisted_states`).
    /// This does not affect the fork RPC cache location.
    pub cache_path: Option<PathBuf>,
    /// Accounts to fund with specific balances on startup (address -> balance in wei).
    pub funded_accounts: HashMap<Address, U256>,
}

impl NodeConfig {
    fn as_string(&self, fork: Option<&ClientFork>) -> String {
        let mut s: String = String::new();
        let _ = write!(s, "\n{}", BANNER.green());
        let _ = write!(s, "\n    {VERSION_MESSAGE}");
        let _ = write!(s, "\n    {}", "https://github.com/foundry-rs/foundry".green());

        let _ = write!(
            s,
            r#"

Available Accounts
==================
"#
        );
        let balance = alloy_primitives::utils::format_ether(self.genesis_balance);
        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            write!(s, "\n({idx}) {} ({balance} ETH)", wallet.address()).unwrap();
        }

        let _ = write!(
            s,
            r#"

Private Keys
==================
"#
        );

        for (idx, wallet) in self.genesis_accounts.iter().enumerate() {
            let hex = hex::encode(wallet.credential().to_bytes());
            let _ = write!(s, "\n({idx}) 0x{hex}");
        }

        if let Some(generator) = &self.account_generator {
            let _ = write!(
                s,
                r#"

Wallet
==================
Mnemonic:          {}
Derivation path:   {}
"#,
                generator.phrase,
                generator.get_derivation_path()
            );
        }

        if let Some(fee_payer) = self.tempo_fee_payer_address() {
            let _ = write!(
                s,
                r#"

Tempo Fee Payer
==================
{fee_payer}
"#
            );
        }

        if let Some(fork) = fork {
            let _ = write!(
                s,
                r#"

Fork
==================
Endpoint:       {}
Block number:   {}
Block hash:     {:?}
Chain ID:       {}
"#,
                fork.eth_rpc_url().as_deref().map(redact_url).unwrap_or_else(|| "none".to_string()),
                fork.block_number(),
                fork.block_hash(),
                fork.execution_chain_id()
            );
            if fork.chain_id() != fork.execution_chain_id() {
                let _ = writeln!(s, "Source chain ID: {}", fork.chain_id());
            }

            if self.fork_urls.len() > 1 {
                let _ = writeln!(s, "Endpoints:      {}", self.fork_urls.len());
                for (i, url) in self.fork_urls.iter().enumerate() {
                    let _ = writeln!(s, "  ({i}) {}", redact_url(url));
                }
            }

            if let Some(tx_hash) = fork.transaction_hash() {
                let _ = writeln!(s, "Transaction hash: {tx_hash}");
            }
        } else {
            let _ = write!(
                s,
                r#"

Chain ID
==================

{}
"#,
                self.get_chain_id().green()
            );
        }

        if (SpecId::from(self.get_hardfork()) as u8) < (SpecId::LONDON as u8) {
            let _ = write!(
                s,
                r#"
Gas Price
==================

{}
"#,
                self.get_gas_price().green()
            );
        } else {
            let _ = write!(
                s,
                r#"
Base Fee
==================

{}
"#,
                self.get_base_fee().green()
            );
        }

        let _ = write!(
            s,
            r#"
Gas Limit
==================

{}
"#,
            {
                if self.disable_block_gas_limit {
                    "Disabled".to_string()
                } else {
                    self.gas_limit.map(|l| l.to_string()).unwrap_or_else(|| {
                        if self.fork_choice.is_some() {
                            "Forked".to_string()
                        } else {
                            DEFAULT_GAS_LIMIT.to_string()
                        }
                    })
                }
            }
            .green()
        );

        let _ = write!(
            s,
            r#"
Genesis Timestamp
==================

{}
"#,
            self.get_genesis_timestamp().green()
        );

        let _ = write!(
            s,
            r#"
Genesis Number
==================

{}
"#,
            self.get_genesis_number().green()
        );

        s
    }

    fn as_json(&self, fork: Option<&ClientFork>) -> Value {
        let mut wallet_description = HashMap::new();
        let mut available_accounts = Vec::with_capacity(self.genesis_accounts.len());
        let mut private_keys = Vec::with_capacity(self.genesis_accounts.len());

        for wallet in &self.genesis_accounts {
            available_accounts.push(format!("{:?}", wallet.address()));
            private_keys.push(format!("0x{}", hex::encode(wallet.credential().to_bytes())));
        }

        if let Some(generator) = &self.account_generator {
            let phrase = generator.get_phrase().to_string();
            let derivation_path = generator.get_derivation_path().to_string();

            wallet_description.insert("derivation_path".to_string(), derivation_path);
            wallet_description.insert("mnemonic".to_string(), phrase);
        };

        let gas_limit = match self.gas_limit {
            // if we have a disabled flag we should max out the limit
            Some(_) | None if self.disable_block_gas_limit => Some(u64::MAX.to_string()),
            Some(limit) => Some(limit.to_string()),
            _ => None,
        };

        if let Some(fork) = fork {
            json!({
              "available_accounts": available_accounts,
              "private_keys": private_keys,
              "endpoint": fork.eth_rpc_url().as_deref().map(redact_url).unwrap_or_default(),
              "block_number": fork.block_number(),
              "block_hash": fork.block_hash(),
              "chain_id": fork.execution_chain_id(),
              "source_chain_id": fork.chain_id(),
              "wallet": wallet_description,
              "base_fee": format!("{}", self.get_base_fee()),
              "gas_price": format!("{}", self.get_gas_price()),
              "gas_limit": gas_limit,
            })
        } else {
            json!({
              "available_accounts": available_accounts,
              "private_keys": private_keys,
              "wallet": wallet_description,
              "base_fee": format!("{}", self.get_base_fee()),
              "gas_price": format!("{}", self.get_gas_price()),
              "gas_limit": gas_limit,
              "genesis_timestamp": format!("{}", self.get_genesis_timestamp()),
            })
        }
    }
}

impl NodeConfig {
    /// Returns a new config intended to be used in tests, which does not print and binds to a
    /// random, free port by setting it to `0`
    #[doc(hidden)]
    pub fn test() -> Self {
        Self { enable_tracing: true, port: 0, silent: true, ..Default::default() }
    }

    /// Returns a test config with Tempo network enabled.
    #[doc(hidden)]
    pub fn test_tempo() -> Self {
        Self { networks: NetworkConfigs::with_tempo(), ..Self::test() }
    }

    /// Returns a test config with Monad network enabled.
    #[cfg(feature = "monad")]
    #[doc(hidden)]
    pub fn test_monad() -> Self {
        Self { networks: NetworkConfigs::with_monad(), ..Self::test() }
    }

    /// Returns a test config with Base network enabled.
    #[cfg(feature = "base")]
    #[doc(hidden)]
    pub fn test_base() -> Self {
        Self::test()
            .with_networks(NetworkConfigs::with_base())
            .with_chain_id(Some(NamedChain::Base as u64))
    }

    /// Returns a new config which does not initialize any accounts on node startup.
    pub fn empty_state() -> Self {
        Self {
            genesis_accounts: vec![],
            signer_accounts: vec![],
            disable_default_create2_deployer: true,
            ..Default::default()
        }
    }
}

impl Default for NodeConfig {
    fn default() -> Self {
        // generate some random wallets
        let genesis_accounts = AccountGenerator::new(10)
            .phrase(DEFAULT_MNEMONIC)
            .generate()
            .expect("Invalid mnemonic.");
        Self {
            chain_id: None,
            gas_limit: None,
            disable_block_gas_limit: false,
            enable_tx_gas_limit: false,
            gas_price: None,
            hardfork: None,
            signer_accounts: genesis_accounts.clone(),
            genesis_timestamp: None,
            genesis_block_number: None,
            genesis_accounts,
            // 100ETH default balance
            genesis_balance: Unit::ETHER.wei().saturating_mul(U256::from(100u64)),
            block_time: None,
            no_mining: false,
            mixed_mining: false,
            port: NODE_PORT,
            max_transactions: 1_000,
            fork_urls: vec![],
            fork_choice: None,
            account_generator: None,
            base_fee: None,
            disable_min_priority_fee: false,
            blob_excess_gas_and_price: None,
            enable_tracing: true,
            enable_steps_tracing: false,
            print_logs: true,
            print_traces: false,
            enable_auto_impersonate: false,
            no_storage_caching: false,
            server_config: Default::default(),
            host: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            transaction_order: Default::default(),
            config_out: None,
            genesis: None,
            fork_request_timeout: REQUEST_TIMEOUT,
            fork_headers: vec![],
            fork_request_retries: 5,
            fork_retry_backoff: Duration::from_millis(1_000),
            fork_chain_id: None,
            fork_source_chain_id: None,
            fork_execution_chain_id: None,
            fork_endpoint_is_anvil: false,
            inferred_fork_network: None,
            chain_id_network_base: None,
            fork_overrides: None,
            // alchemy max cpus <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
            compute_units_per_second: ALCHEMY_FREE_TIER_CUPS,
            ipc_path: None,
            code_size_limit: None,
            prune_history: Default::default(),
            max_persisted_states: None,
            init_state: None,
            transaction_block_keeper: None,
            disable_default_create2_deployer: false,
            disable_pool_balance_checks: false,
            slots_in_an_epoch: DEFAULT_SLOTS_IN_AN_EPOCH,
            memory_limit: None,
            precompile_factory: None,
            networks: Default::default(),
            #[cfg(feature = "base")]
            base_activation_admin: None,
            tempo_fee_payer: None,
            silent: false,
            cache_path: None,
            funded_accounts: HashMap::default(),
        }
    }
}

impl NodeConfig {
    /// Applies Tempo's safe default beneficiary for forked nodes while preserving
    /// explicit coinbase selections.
    pub(crate) fn apply_tempo_fork_beneficiary_default<N>(&self, evm_env: &mut EvmEnv<N>) {
        if self.networks.is_tempo()
            && !self.fork_urls.is_empty()
            && evm_env.block_env.beneficiary.is_zero()
        {
            // Tempo mainnet maps the zero validator token to a DONOTUSE sentinel.
            // Forked transactions with the default zero beneficiary can therefore
            // fail fee collection before producing a receipt. Use the same neutral
            // fee-recipient sentinel as Tempo's simulation path so validator token
            // lookup falls back to the default PathUSD token unless the user has
            // explicitly supplied a non-zero coinbase.
            evm_env.block_env.beneficiary = TIP_FEE_MANAGER_ADDRESS;
        }
    }

    /// Returns the memory limit of the node
    #[must_use]
    pub const fn with_memory_limit(mut self, mems_value: Option<u64>) -> Self {
        self.memory_limit = mems_value;
        self
    }

    /// Returns the base fee to use.
    ///
    /// In Tempo mode, uses the hardfork-specific base fee (10 gwei pre-T1, 20 gwei T1+).
    pub fn get_base_fee(&self) -> u64 {
        let default = if self.networks.is_tempo() {
            tempo_default_base_fee(TempoHardfork::from(self.get_hardfork()))
        } else {
            INITIAL_BASE_FEE
        };
        self.base_fee
            .or_else(|| self.genesis.as_ref().and_then(|g| g.base_fee_per_gas.map(|g| g as u64)))
            .unwrap_or(default)
    }

    /// Returns the gas price to use.
    ///
    /// In Tempo mode, defaults to the hardfork-specific base fee.
    pub fn get_gas_price(&self) -> u128 {
        let default = if self.networks.is_tempo() {
            tempo_default_base_fee(TempoHardfork::from(self.get_hardfork())) as u128
        } else {
            INITIAL_GAS_PRICE
        };
        self.gas_price.unwrap_or(default)
    }

    pub fn get_blob_excess_gas_and_price(&self) -> BlobExcessGasAndPrice {
        if let Some(value) = self.blob_excess_gas_and_price {
            value
        } else {
            let excess_blob_gas =
                self.genesis.as_ref().and_then(|g| g.excess_blob_gas).unwrap_or(0);
            BlobExcessGasAndPrice::new(
                excess_blob_gas,
                self.get_blob_params().update_fraction as u64,
            )
        }
    }

    /// Returns the [`BlobParams`] that should be used.
    pub fn get_blob_params(&self) -> BlobParams {
        get_blob_params_by_hardfork(self.get_hardfork())
    }

    /// Returns the hardfork to use
    pub fn get_hardfork(&self) -> FoundryHardfork {
        if let Some(hardfork) = self.hardfork {
            return hardfork;
        }
        self.networks
            .execution_network()
            .hardfork_at(self.protocol_chain_id(), self.get_genesis_timestamp())
    }

    /// Sets a custom code size limit
    #[must_use]
    pub const fn with_code_size_limit(mut self, code_size_limit: Option<usize>) -> Self {
        self.code_size_limit = code_size_limit;
        self
    }
    /// Disables  code size limit
    #[must_use]
    pub const fn disable_code_size_limit(mut self, disable_code_size_limit: bool) -> Self {
        if disable_code_size_limit {
            self.code_size_limit = Some(usize::MAX);
        }
        self
    }

    /// Sets the init state if any
    #[must_use]
    pub fn with_init_state(mut self, init_state: Option<SerializableState>) -> Self {
        self.init_state = init_state;
        self
    }

    /// Loads the init state from a file if it exists
    #[must_use]
    #[cfg(feature = "cmd")]
    pub fn with_init_state_path(mut self, path: impl AsRef<std::path::Path>) -> Self {
        self.init_state = crate::cmd::StateFile::parse_path(path).ok().and_then(|file| file.state);
        self
    }

    /// Sets the chain ID
    #[must_use]
    pub fn with_chain_id<U: Into<u64>>(mut self, chain_id: Option<U>) -> Self {
        self.set_chain_id(chain_id);
        self
    }

    /// Returns the chain ID to use
    pub fn get_chain_id(&self) -> u64 {
        self.chain_id
            .or(self.fork_execution_chain_id)
            .or_else(|| self.genesis.as_ref().map(|g| g.config.chain_id))
            .unwrap_or(CHAIN_ID)
    }

    /// Returns the chain ID that defines protocol behavior.
    fn protocol_chain_id(&self) -> u64 {
        self.fork_source_chain_id.unwrap_or_else(|| self.get_chain_id())
    }

    /// Sets the chain id and updates all wallets
    pub fn set_chain_id(&mut self, chain_id: Option<impl Into<u64>>) {
        if let Some(base) = self.chain_id_network_base.take() {
            self.networks = base;
        }
        self.chain_id = chain_id.map(Into::into);
        let chain_id = self.get_chain_id();
        let base = self.networks;
        let inferred = base.with_chain_id(chain_id);
        if !base.has_network_selection() && inferred.has_network_selection() {
            self.chain_id_network_base = Some(base);
        }
        self.networks = inferred;
        self.update_wallet_chain_id(chain_id);
    }

    pub(crate) fn update_wallet_chain_id(&mut self, chain_id: u64) {
        self.genesis_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(Some(chain_id));
        });
        self.signer_accounts.iter_mut().for_each(|wallet| {
            *wallet = wallet.clone().with_chain_id(Some(chain_id));
        })
    }

    /// Sets the gas limit
    #[must_use]
    pub const fn with_gas_limit(mut self, gas_limit: Option<u64>) -> Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Disable block gas limit check
    ///
    /// If set to `true` block gas limit will not be enforced
    #[must_use]
    pub const fn disable_block_gas_limit(mut self, disable_block_gas_limit: bool) -> Self {
        self.disable_block_gas_limit = disable_block_gas_limit;
        self
    }

    /// Enable tx gas limit check
    ///
    /// If set to `true`, enables the tx gas limit as imposed by Osaka (EIP-7825)
    #[must_use]
    pub const fn enable_tx_gas_limit(mut self, enable_tx_gas_limit: bool) -> Self {
        self.enable_tx_gas_limit = enable_tx_gas_limit;
        self
    }

    /// Sets the gas price
    #[must_use]
    pub const fn with_gas_price(mut self, gas_price: Option<u128>) -> Self {
        self.gas_price = gas_price;
        self
    }

    /// Sets prune history status.
    #[must_use]
    pub fn set_pruned_history(mut self, prune_history: Option<Option<usize>>) -> Self {
        self.prune_history = PruneStateHistoryConfig::from_args(prune_history);
        self
    }

    /// Sets max number of states to cache on disk.
    #[must_use]
    pub fn with_max_persisted_states<U: Into<usize>>(
        mut self,
        max_persisted_states: Option<U>,
    ) -> Self {
        self.max_persisted_states = max_persisted_states.map(Into::into);
        self
    }

    /// Sets the max number of transactions in a block
    #[must_use]
    pub const fn with_max_transactions(mut self, max_transactions: Option<usize>) -> Self {
        if let Some(max_transactions) = max_transactions {
            self.max_transactions = max_transactions;
        }
        self
    }

    /// Sets max number of blocks with transactions to keep in memory
    #[must_use]
    pub fn with_transaction_block_keeper<U: Into<usize>>(
        mut self,
        transaction_block_keeper: Option<U>,
    ) -> Self {
        self.transaction_block_keeper = transaction_block_keeper.map(Into::into);
        self
    }

    /// Sets the base fee
    #[must_use]
    pub const fn with_base_fee(mut self, base_fee: Option<u64>) -> Self {
        self.base_fee = base_fee;
        self
    }

    /// Disable the enforcement of a minimum suggested priority fee
    #[must_use]
    pub const fn disable_min_priority_fee(mut self, disable_min_priority_fee: bool) -> Self {
        self.disable_min_priority_fee = disable_min_priority_fee;
        self
    }

    /// Sets the init genesis (genesis.json)
    #[must_use]
    pub fn with_genesis(mut self, genesis: Option<Genesis>) -> Self {
        self.genesis = genesis;
        self
    }

    /// Returns the genesis timestamp to use
    pub fn get_genesis_timestamp(&self) -> u64 {
        self.genesis_timestamp
            .or_else(|| self.genesis.as_ref().map(|g| g.timestamp))
            .unwrap_or_else(|| duration_since_unix_epoch().as_secs())
    }

    /// Sets the genesis timestamp
    #[must_use]
    pub fn with_genesis_timestamp<U: Into<u64>>(mut self, timestamp: Option<U>) -> Self {
        if let Some(timestamp) = timestamp {
            self.genesis_timestamp = Some(timestamp.into());
        }
        self
    }

    /// Sets the genesis number
    #[must_use]
    pub fn with_genesis_block_number<U: Into<u64>>(mut self, number: Option<U>) -> Self {
        if let Some(number) = number {
            self.genesis_block_number = Some(number.into());
        }
        self
    }

    /// Returns the genesis number
    pub fn get_genesis_number(&self) -> u64 {
        self.genesis_block_number
            .or_else(|| self.genesis.as_ref().and_then(|g| g.number))
            .unwrap_or(0)
    }

    /// Sets the hardfork
    #[must_use]
    pub const fn with_hardfork(mut self, hardfork: Option<FoundryHardfork>) -> Self {
        self.hardfork = hardfork;
        self
    }

    /// Sets the genesis accounts
    #[must_use]
    pub fn with_genesis_accounts(mut self, accounts: Vec<PrivateKeySigner>) -> Self {
        self.genesis_accounts = accounts;
        self
    }

    /// Sets the signer accounts
    #[must_use]
    pub fn with_signer_accounts(mut self, accounts: Vec<PrivateKeySigner>) -> Self {
        self.signer_accounts = accounts;
        self
    }

    /// Sets both the genesis accounts and the signer accounts
    /// so that `genesis_accounts == accounts`
    pub fn with_account_generator(mut self, generator: AccountGenerator) -> eyre::Result<Self> {
        let accounts = generator.generate()?;
        self.account_generator = Some(generator);
        Ok(self.with_signer_accounts(accounts.clone()).with_genesis_accounts(accounts))
    }

    /// Sets the balance of the genesis accounts in the genesis block
    #[must_use]
    pub fn with_genesis_balance<U: Into<U256>>(mut self, balance: U) -> Self {
        self.genesis_balance = balance.into();
        self
    }

    /// Sets the block time to automine blocks
    #[must_use]
    pub fn with_blocktime<D: Into<Duration>>(mut self, block_time: Option<D>) -> Self {
        self.block_time = block_time.map(Into::into);
        self
    }

    #[must_use]
    pub fn with_mixed_mining<D: Into<Duration>>(
        mut self,
        mixed_mining: bool,
        block_time: Option<D>,
    ) -> Self {
        self.block_time = block_time.map(Into::into);
        self.mixed_mining = mixed_mining;
        self
    }

    /// If set to `true` auto mining will be disabled
    #[must_use]
    pub const fn with_no_mining(mut self, no_mining: bool) -> Self {
        self.no_mining = no_mining;
        self
    }

    /// Sets the slots in an epoch
    #[must_use]
    pub const fn with_slots_in_an_epoch(mut self, slots_in_an_epoch: u64) -> Self {
        self.slots_in_an_epoch = slots_in_an_epoch;
        self
    }

    /// Sets the port to use
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the ipc path to use
    ///
    /// Note: this is a double Option for
    ///     - `None` -> no ipc
    ///     - `Some(None)` -> use default path
    ///     - `Some(Some(path))` -> use custom path
    #[must_use]
    pub fn with_ipc(mut self, ipc_path: Option<Option<String>>) -> Self {
        self.ipc_path = ipc_path;
        self
    }

    /// Sets the file path to write the Anvil node's config info to.
    #[must_use]
    pub fn set_config_out(mut self, config_out: Option<PathBuf>) -> Self {
        self.config_out = config_out;
        self
    }

    #[must_use]
    pub const fn with_no_storage_caching(mut self, no_storage_caching: bool) -> Self {
        self.no_storage_caching = no_storage_caching;
        self
    }

    /// Sets the `eth_rpc_url` to use when forking (single endpoint convenience).
    #[must_use]
    pub fn with_eth_rpc_url<U: Into<String>>(mut self, eth_rpc_url: Option<U>) -> Self {
        if let Some(url) = eth_rpc_url {
            let fork_urls = vec![url.into()];
            if self.fork_urls != fork_urls {
                self.fork_endpoint_is_anvil = false;
            }
            self.fork_urls = fork_urls;
        }
        self
    }

    /// Sets the fork URLs for load-balanced multi-endpoint forking.
    #[must_use]
    pub fn with_fork_urls(mut self, fork_urls: Vec<String>) -> Self {
        if self.fork_urls != fork_urls {
            self.fork_endpoint_is_anvil = false;
        }
        self.fork_urls = fork_urls;
        self
    }

    /// Sets the `fork_choice` to use to fork off from based on a block number
    #[must_use]
    pub fn with_fork_block_number<U: Into<u64>>(self, fork_block_number: Option<U>) -> Self {
        self.with_fork_choice(fork_block_number.map(Into::into))
    }

    /// Sets the `fork_choice` to use to fork off from based on a transaction hash
    #[must_use]
    pub fn with_fork_transaction_hash<U: Into<TxHash>>(
        self,
        fork_transaction_hash: Option<U>,
    ) -> Self {
        self.with_fork_choice(fork_transaction_hash.map(Into::into))
    }

    /// Sets the `fork_choice` to use to fork off from
    #[must_use]
    pub fn with_fork_choice<U: Into<ForkChoice>>(mut self, fork_choice: Option<U>) -> Self {
        self.fork_choice = fork_choice.map(Into::into);
        self
    }

    /// Sets the `fork_chain_id` to use to fork off local cache from
    #[must_use]
    pub const fn with_fork_chain_id(mut self, fork_chain_id: Option<U256>) -> Self {
        self.fork_chain_id = fork_chain_id;
        self
    }

    /// Sets the `fork_headers` to use with fork RPC endpoints
    #[must_use]
    pub fn with_fork_headers(mut self, headers: Vec<String>) -> Self {
        self.fork_headers = headers;
        self
    }

    /// Sets the `fork_request_timeout` to use for requests
    #[must_use]
    pub const fn fork_request_timeout(mut self, fork_request_timeout: Option<Duration>) -> Self {
        if let Some(fork_request_timeout) = fork_request_timeout {
            self.fork_request_timeout = fork_request_timeout;
        }
        self
    }

    /// Sets the `fork_request_retries` to use for spurious networks
    #[must_use]
    pub const fn fork_request_retries(mut self, fork_request_retries: Option<u32>) -> Self {
        if let Some(fork_request_retries) = fork_request_retries {
            self.fork_request_retries = fork_request_retries;
        }
        self
    }

    /// Sets the initial `fork_retry_backoff` for rate limits
    #[must_use]
    pub const fn fork_retry_backoff(mut self, fork_retry_backoff: Option<Duration>) -> Self {
        if let Some(fork_retry_backoff) = fork_retry_backoff {
            self.fork_retry_backoff = fork_retry_backoff;
        }
        self
    }

    /// Sets the number of assumed available compute units per second
    ///
    /// See also, <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    #[must_use]
    pub const fn fork_compute_units_per_second(
        mut self,
        compute_units_per_second: Option<u64>,
    ) -> Self {
        if let Some(compute_units_per_second) = compute_units_per_second {
            self.compute_units_per_second = compute_units_per_second;
        }
        self
    }

    /// Sets whether to enable tracing
    #[must_use]
    pub const fn with_tracing(mut self, enable_tracing: bool) -> Self {
        self.enable_tracing = enable_tracing;
        self
    }

    /// Sets whether to enable steps tracing
    #[must_use]
    pub const fn with_steps_tracing(mut self, enable_steps_tracing: bool) -> Self {
        self.enable_steps_tracing = enable_steps_tracing;
        self
    }

    /// Sets whether to print `console.log` invocations to stdout.
    #[must_use]
    pub const fn with_print_logs(mut self, print_logs: bool) -> Self {
        self.print_logs = print_logs;
        self
    }

    /// Sets whether to print traces to stdout.
    #[must_use]
    pub const fn with_print_traces(mut self, print_traces: bool) -> Self {
        self.print_traces = print_traces;
        self
    }

    /// Sets whether to enable autoImpersonate
    #[must_use]
    pub const fn with_auto_impersonate(mut self, enable_auto_impersonate: bool) -> Self {
        self.enable_auto_impersonate = enable_auto_impersonate;
        self
    }

    #[must_use]
    pub fn with_server_config(mut self, config: ServerConfig) -> Self {
        self.server_config = config;
        self
    }

    /// Sets the host the server will listen on
    #[must_use]
    pub fn with_host(mut self, host: Vec<IpAddr>) -> Self {
        self.host = if host.is_empty() { vec![IpAddr::V4(Ipv4Addr::LOCALHOST)] } else { host };
        self
    }

    #[must_use]
    pub const fn with_transaction_order(mut self, transaction_order: TransactionOrder) -> Self {
        self.transaction_order = transaction_order;
        self
    }

    /// Returns the ipc path for the ipc endpoint if any
    pub fn get_ipc_path(&self) -> Option<String> {
        match &self.ipc_path {
            Some(path) => path.clone().or_else(|| Some(DEFAULT_IPC_ENDPOINT.to_string())),
            None => None,
        }
    }

    /// Prints the config info
    pub fn print(&self, fork: Option<&ClientFork>) -> Result<()> {
        if let Some(path) = &self.config_out {
            let value = self.as_json(fork);
            foundry_common::fs::write_json_file(path, &value).wrap_err("failed writing JSON")?;
        }
        if !self.silent {
            sh_println!("{}", self.as_string(fork))?;
        }
        Ok(())
    }

    /// Returns the endpoint-specific path where the cache file should be stored.
    ///
    /// See also [`Config::foundry_block_cache_file`].
    pub fn block_cache_path(&self, block: u64) -> Option<PathBuf> {
        self.block_cache_path_for_rpc(self.protocol_chain_id(), block, self.fork_urls.first()?)
    }

    fn block_cache_path_for_rpc(
        &self,
        source_chain_id: u64,
        block: u64,
        rpc_url: &str,
    ) -> Option<PathBuf> {
        if self.no_storage_caching || self.fork_urls.is_empty() {
            return None;
        }

        let rpc_url_hash = hex::encode(keccak256(rpc_url));
        Some(
            Config::foundry_block_cache_file(source_chain_id, block)?
                .with_file_name(format!("storage-{rpc_url_hash}.json")),
        )
    }

    /// Sets whether to disable the default create2 deployer
    #[must_use]
    pub const fn with_disable_default_create2_deployer(mut self, yes: bool) -> Self {
        self.disable_default_create2_deployer = yes;
        self
    }

    /// Sets whether to disable pool balance checks
    #[must_use]
    pub const fn with_disable_pool_balance_checks(mut self, yes: bool) -> Self {
        self.disable_pool_balance_checks = yes;
        self
    }

    /// Injects precompiles to `anvil`'s EVM.
    #[must_use]
    pub fn with_precompile_factory(mut self, factory: impl PrecompileFactory + 'static) -> Self {
        self.precompile_factory = Some(Arc::new(factory));
        self
    }

    /// Enable features for provided networks.
    #[must_use]
    pub const fn with_networks(mut self, networks: NetworkConfigs) -> Self {
        self.networks = networks;
        self.inferred_fork_network = None;
        self.chain_id_network_base = None;
        self
    }

    /// Enable Tempo network features.
    #[must_use]
    pub fn with_tempo(mut self) -> Self {
        self.networks = NetworkConfigs::with_tempo();
        self.inferred_fork_network = None;
        self.chain_id_network_base = None;
        self
    }

    /// Sets the account used to sponsor Tempo fee-payer requests.
    #[must_use]
    pub const fn with_tempo_fee_payer(mut self, fee_payer: Option<Address>) -> Self {
        self.tempo_fee_payer = fee_payer;
        self
    }

    /// Returns the effective account used to sponsor Tempo fee-payer requests.
    ///
    /// Defaults to the last dev account so it rarely collides with the sender accounts commonly
    /// used in tests, mirroring the dedicated sponsor account of hosted fee payer services.
    /// Returns `None` on non-Tempo networks.
    pub fn tempo_fee_payer_address(&self) -> Option<Address> {
        if !self.networks.is_tempo() {
            return None;
        }
        self.tempo_fee_payer.or_else(|| self.genesis_accounts.last().map(|wallet| wallet.address()))
    }

    /// Enable Monad network features.
    #[cfg(feature = "monad")]
    #[must_use]
    pub fn with_monad(mut self) -> Self {
        self.networks = NetworkConfigs::with_monad();
        self.inferred_fork_network = None;
        self.chain_id_network_base = None;
        self
    }

    /// Enable Base network features.
    #[cfg(feature = "base")]
    #[must_use]
    pub fn with_base(mut self) -> Self {
        self.networks = NetworkConfigs::with_base();
        self.inferred_fork_network = None;
        self.chain_id_network_base = None;
        self
    }

    /// Sets the Base activation-registry administrator override.
    #[cfg(feature = "base")]
    #[must_use]
    pub const fn with_base_activation_admin(mut self, admin: Option<Address>) -> Self {
        self.base_activation_admin = admin;
        self
    }

    /// Enable Optimism network features.
    #[cfg(feature = "optimism")]
    #[must_use]
    pub fn with_optimism(mut self) -> Self {
        self.networks = NetworkConfigs::with_optimism();
        self.inferred_fork_network = None;
        self.chain_id_network_base = None;
        self
    }

    /// Makes the node silent to not emit anything on stdout
    #[must_use]
    pub const fn silent(self) -> Self {
        self.set_silent(true)
    }

    #[must_use]
    pub const fn set_silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }

    /// Sets the path where persisted states are cached (used with `max_persisted_states`).
    ///
    /// Note: This does not control the fork RPC cache location, which uses endpoint-specific files
    /// under `~/.foundry/cache/rpc/<chain>/<block>/`.
    #[must_use]
    pub fn with_cache_path(mut self, cache_path: Option<PathBuf>) -> Self {
        self.cache_path = cache_path;
        self
    }

    /// Sets accounts to fund with custom balances on startup.
    #[must_use]
    pub fn with_funded_accounts(mut self, accounts: HashMap<Address, U256>) -> Self {
        self.funded_accounts = accounts;
        self
    }

    /// Configures everything related to env, backend and database and returns the
    /// [Backend](mem::Backend)
    ///
    /// *Note*: only memory based backend for now
    pub(crate) async fn setup<N>(
        &mut self,
    ) -> Result<(mem::Backend<N>, Option<ForkTransactionReplay>)>
    where
        N: alloy_network::Network<
                TxEnvelope = foundry_primitives::FoundryTxEnvelope,
                ReceiptEnvelope = foundry_primitives::FoundryReceiptEnvelope,
            >,
    {
        // configure the revm environment

        let mut cfg = CfgEnv::default();
        cfg.spec = self.get_hardfork().into();

        cfg.chain_id = self.get_chain_id();
        cfg.limit_contract_code_size = self.code_size_limit;
        // EIP-3607 rejects transactions from senders with deployed code.
        // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the
        // caller is a contract. So we disable the check by default.
        cfg.disable_eip3607 = true;
        cfg.disable_block_gas_limit = self.disable_block_gas_limit;

        if !self.enable_tx_gas_limit {
            cfg.tx_gas_limit_cap = Some(u64::MAX);
        }

        if let Some(value) = self.memory_limit {
            cfg.memory_limit = value;
        }

        let spec_id = cfg.spec;
        let mut evm_env = EvmEnv::new(
            cfg,
            BlockEnv {
                gas_limit: self.gas_limit(),
                basefee: self.get_base_fee(),
                ..Default::default()
            },
        );

        self.apply_tempo_fork_beneficiary_default(&mut evm_env);

        let genesis_timestamp = self.get_genesis_timestamp();
        let base_fee_params: BaseFeeParams = self.networks.base_fee_params(genesis_timestamp);

        // On Tempo, the base fee follows the chain's hardfork rules instead of EIP-1559.
        let tempo_hardfork =
            self.networks.is_tempo().then(|| TempoHardfork::from(self.get_hardfork()));

        let fees = FeeManager::new(
            spec_id,
            self.get_base_fee(),
            !self.disable_min_priority_fee,
            self.get_gas_price(),
            self.get_blob_excess_gas_and_price(),
            self.get_blob_params(),
            base_fee_params,
            tempo_hardfork,
        );
        #[cfg(feature = "optimism")]
        if self.networks.is_optimism() {
            fees.set_optimism_hardfork(self.get_hardfork());
        }

        let (db, fork, fork_transaction_replay) =
            if let Some(eth_rpc_url) = self.fork_urls.first().cloned() {
                self.setup_fork_db_with_replay(eth_rpc_url, &mut evm_env, &fees).await?
            } else {
                let track_history = self.prune_history.is_state_history_supported();
                let db: Arc<TokioRwLock<Box<dyn Db>>> =
                    Arc::new(TokioRwLock::new(Box::new(StateRootDb::new(track_history))));
                (db, None, None)
            };

        // if provided use all settings of `genesis.json`
        if let Some(ref genesis) = self.genesis {
            // --chain-id flag gets precedence over the genesis.json chain id
            // <https://github.com/foundry-rs/foundry/issues/10059>
            if self.chain_id.is_none() && fork.is_none() {
                evm_env.cfg_env.chain_id = genesis.config.chain_id;
            }
            evm_env.block_env.timestamp = U256::from(genesis.timestamp);
            if let Some(base_fee) = genesis.base_fee_per_gas {
                evm_env.block_env.basefee = base_fee.try_into()?;
            }
            if let Some(number) = genesis.number {
                evm_env.block_env.number = U256::from(number);
            }
            evm_env.block_env.beneficiary = genesis.coinbase;
        }

        // Fork setup initializes its own timestamp. For a local BSC chain, keep the initial EVM
        // and genesis block on the same resolved timestamp so chain precompiles are available
        // immediately. Preserve the default timestamp behavior for all other local chains.
        let is_bsc = matches!(
            NamedChain::try_from(evm_env.cfg_env.chain_id),
            Ok(NamedChain::BinanceSmartChain | NamedChain::BinanceSmartChainTestnet)
        );
        if fork.is_none() && (self.genesis_timestamp.is_some() || is_bsc) {
            evm_env.block_env.timestamp = U256::from(genesis_timestamp);
        }

        self.apply_tempo_fork_beneficiary_default(&mut evm_env);

        let genesis = GenesisConfig {
            number: self.get_genesis_number(),
            timestamp: genesis_timestamp,
            balance: self.genesis_balance,
            accounts: self.genesis_accounts.iter().map(|acc| acc.address()).collect(),
            genesis_init: self.genesis.clone(),
        };

        let active_hardfork = fork
            .as_ref()
            .and_then(|fork| fork.config.read().hardfork)
            .unwrap_or_else(|| self.get_hardfork());
        let mut decoder_builder = CallTraceDecoderBuilder::new()
            .with_networks(self.networks)
            .with_hardfork(Some(self.networks.executed_hardfork(active_hardfork)));
        if self.print_traces {
            // if traces should get printed we configure the decoder with the signatures cache
            if let Ok(identifier) = SignaturesIdentifier::new(false) {
                debug!(target: "node", "using signature identifier");
                decoder_builder = decoder_builder.with_signature_identifier(identifier);
            }
        }

        // only memory based backend for now
        let backend = mem::Backend::with_genesis(
            db,
            Arc::new(RwLock::new(evm_env)),
            self.networks,
            genesis,
            fees,
            Arc::new(RwLock::new(fork)),
            self.enable_steps_tracing,
            self.print_logs,
            self.print_traces,
            Arc::new(decoder_builder.build()),
            self.prune_history,
            self.max_persisted_states,
            self.transaction_block_keeper,
            self.block_time,
            self.cache_path.clone(),
            Arc::new(TokioRwLock::new(self.clone())),
        )
        .await?;

        // Writes the default create2 deployer to the backend,
        // if the option is not disabled and we are not forking.
        if !self.disable_default_create2_deployer && self.fork_urls.is_empty() {
            backend
                .set_create2_deployer(DEFAULT_CREATE2_DEPLOYER)
                .await
                .wrap_err("failed to create default create2 deployer")?;
        }

        if let Some(fork) = backend.get_fork() {
            let config = fork.config.read().clone();
            if !self
                .fork_urls_match_context(
                    &config.fork_urls,
                    config.endpoint_identity,
                    config.block_number,
                    config.block_hash,
                )
                .await?
            {
                eyre::bail!("fork endpoint changed while Anvil was being initialized");
            }
        }
        Ok((backend, fork_transaction_replay))
    }

    /// Configures everything related to forking based on the passed `eth_rpc_url`:
    ///  - returning a tuple of a [ForkedDatabase] wrapped in an [Arc] [RwLock](TokioRwLock) and
    ///    [ClientFork] wrapped in an [Option] which can be used in a [Backend](mem::Backend) to
    ///    fork from.
    ///  - modifying some parameters of the passed `env`
    ///  - mutating some members of `self`
    pub async fn setup_fork_db(
        &mut self,
        eth_rpc_url: String,
        evm_env: &mut EvmEnv,
        fees: &FeeManager,
    ) -> Result<(Arc<TokioRwLock<Box<dyn Db>>>, Option<ClientFork>)> {
        let (db, fork, replay) = self.setup_fork_db_with_replay(eth_rpc_url, evm_env, fees).await?;
        eyre::ensure!(replay.is_none(), "transaction-hash fork replay requires full node startup");
        Ok((db, fork))
    }

    async fn setup_fork_db_with_replay(
        &mut self,
        eth_rpc_url: String,
        evm_env: &mut EvmEnv,
        fees: &FeeManager,
    ) -> Result<(Arc<TokioRwLock<Box<dyn Db>>>, Option<ClientFork>, Option<ForkTransactionReplay>)>
    {
        let (db, config, replay) =
            self.setup_fork_db_config_with_replay(eth_rpc_url, evm_env, fees).await?;
        let db: Arc<TokioRwLock<Box<dyn Db>>> = Arc::new(TokioRwLock::new(Box::new(db)));
        let fork = ClientFork::new(config, Arc::clone(&db));
        Ok((db, Some(fork), replay))
    }

    fn fork_provider(&self, eth_rpc_url: &str) -> Result<RetryProvider> {
        ProviderBuilder::new(eth_rpc_url)
            .timeout(self.fork_request_timeout)
            .initial_backoff(self.fork_retry_backoff.as_millis() as u64)
            .compute_units_per_second(self.compute_units_per_second)
            .max_retry(self.fork_request_retries)
            .headers(self.fork_headers.clone())
            .build()
            .wrap_err("failed to establish provider to fork url")
    }

    async fn fork_endpoint_identity(
        &self,
        provider: &RetryProvider,
        fallback_execution_chain_id: u64,
        source_chain_id_override: Option<u64>,
        node_info_probe: &mut AnvilNodeInfoProbe,
    ) -> Result<ForkEndpointIdentity> {
        let Some(node_info) = node_info_probe.request(provider).await? else {
            let source_chain_id = source_chain_id_override.unwrap_or(fallback_execution_chain_id);
            let explicit_fallback = self.has_explicit_network_selection().then_some(self.networks);
            let network_profile = NetworkConfigs::from_rpc_identity_profile_with_fallback(
                source_chain_id,
                None,
                explicit_fallback,
            )
            .map_err(eyre::Report::msg)?;
            return Ok(ForkEndpointIdentity {
                execution_chain_id: fallback_execution_chain_id,
                source_chain_id,
                network: network_profile.map(|profile| profile.execution_network()),
                network_profile,
                hardfork: None,
                instance_id: None,
                source_fork_block_number: None,
                source_fork_block_hash: None,
            });
        };

        let (
            execution_chain_id,
            source_chain_id,
            instance_id,
            source_fork_block_number,
            source_fork_block_hash,
        ) = match provider.raw_request::<_, Metadata>("anvil_metadata".into(), ()).await {
            Ok(metadata) => (
                metadata.chain_id,
                source_chain_id_override.unwrap_or_else(|| {
                    metadata.forked_network.map(|fork| fork.chain_id).unwrap_or(metadata.chain_id)
                }),
                Some(metadata.instance_id),
                metadata.forked_network.map(|fork| fork.fork_block_number),
                metadata.forked_network.map(|fork| fork.fork_block_hash),
            ),
            Err(error) if is_rpc_method_not_found(&error) => (
                fallback_execution_chain_id,
                source_chain_id_override.unwrap_or(fallback_execution_chain_id),
                None,
                None,
                None,
            ),
            Err(error) => {
                return Err(error).wrap_err("failed to retrieve Anvil fork source identity");
            }
        };
        let identity_chain_id =
            if node_info.network.is_some() { execution_chain_id } else { source_chain_id };
        let explicit_fallback = self.has_explicit_network_selection().then_some(self.networks);
        let network_profile = NetworkConfigs::from_rpc_identity_profile_with_fallback(
            identity_chain_id,
            Some(node_info.network.as_deref()),
            explicit_fallback,
        )
        .map_err(eyre::Report::msg)?
        .ok_or_else(|| eyre::eyre!("Anvil metadata did not identify an execution profile"))?;
        let network = network_profile.execution_network();
        let hardfork = network
            .parse_hardfork(&node_info.hard_fork)
            .map_err(eyre::Report::msg)
            .wrap_err_with(|| {
                format!("unsupported hardfork `{}` reported for `{network}`", node_info.hard_fork)
            })?;

        Ok(ForkEndpointIdentity {
            execution_chain_id,
            source_chain_id,
            network: Some(network),
            network_profile: Some(network_profile),
            hardfork: Some(hardfork),
            instance_id,
            source_fork_block_number,
            source_fork_block_hash,
        })
    }

    async fn resolved_fork_endpoint_identity(
        &self,
        provider: &RetryProvider,
        node_info_probe: &mut AnvilNodeInfoProbe,
    ) -> Result<ForkEndpointIdentity> {
        let identity = if let Some(chain_id) = self.fork_chain_id {
            let chain_id = chain_id.to();
            // `fork_chain_id` avoids depending on `eth_chainId`, but an online Anvil endpoint can
            // still expose authoritative family, hardfork, and instance metadata. Probe it so
            // mirror validation and reset staging cannot mistake this node for its own upstream.
            self.fork_endpoint_identity(provider, chain_id, Some(chain_id), node_info_probe).await
        } else {
            let execution_chain_id =
                provider.get_chain_id().await.wrap_err("failed to fetch network chain ID")?;
            self.fork_endpoint_identity(provider, execution_chain_id, None, node_info_probe).await
        }?;
        ensure_fork_network_supported(identity.source_chain_id)?;
        Ok(identity)
    }

    pub(crate) async fn replacement_fork_provider(
        &self,
        eth_rpc_url: &str,
        expected: ForkEndpointIdentity,
        block_number: u64,
        block_hash: B256,
        serving_instance_id: B256,
    ) -> Result<(Arc<RetryProvider>, ForkEndpointIdentity)> {
        let provider = Arc::new(self.fork_provider(eth_rpc_url)?);
        let mut node_info_probe = AnvilNodeInfoProbe::default();
        for _ in 0..3 {
            let before =
                self.resolved_fork_endpoint_identity(&provider, &mut node_info_probe).await?;
            eyre::ensure!(
                before.instance_id != Some(serving_instance_id),
                "cannot set Anvil's fork provider to its own RPC endpoint"
            );
            let block = provider
                .get_block(BlockNumberOrTag::Number(block_number).into())
                .await
                .wrap_err("failed to confirm active fork block on replacement endpoint")?;
            let after =
                self.resolved_fork_endpoint_identity(&provider, &mut node_info_probe).await?;
            if before != after {
                continue;
            }
            if !before.context_eq(expected) {
                eyre::bail!("replacement fork endpoint has an incompatible execution context");
            }
            let actual_hash = block.map(|block| block.header.hash);
            if actual_hash != Some(block_hash) {
                eyre::bail!(
                    "replacement fork endpoint does not contain active fork block {block_number} with hash {block_hash}"
                );
            }
            return Ok((provider, before));
        }
        eyre::bail!(
            "fork endpoint changed while its identity and active fork block were being resolved"
        );
    }

    async fn stable_fork_snapshot(
        &self,
        provider: &Arc<RetryProvider>,
        fork_overrides: ForkOverrides,
    ) -> Result<StableForkSnapshot> {
        let mut node_info_probe = AnvilNodeInfoProbe::new(self.fork_endpoint_is_anvil);
        for _ in 0..3 {
            let before =
                self.resolved_fork_endpoint_identity(provider, &mut node_info_probe).await?;
            let (block_number, transaction_replay) = if let Some(fork_choice) = &self.fork_choice {
                derive_block_and_replay(fork_choice, provider).await.wrap_err(
                    "failed to derive fork block and transaction replay from fork choice",
                )?
            } else {
                (
                    find_latest_fork_block(provider)
                        .await
                        .wrap_err("failed to get fork block number")?,
                    None,
                )
            };
            let block = provider
                .get_block(BlockNumberOrTag::Number(block_number).into())
                .await
                .wrap_err("failed to get fork block")?;
            let gas_price = if let Some(gas_price) = fork_overrides.gas_price {
                gas_price
            } else {
                provider.get_gas_price().await.unwrap_or(INITIAL_GAS_PRICE)
            };
            let after =
                self.resolved_fork_endpoint_identity(provider, &mut node_info_probe).await?;
            if before == after {
                return Ok(StableForkSnapshot {
                    endpoint_identity: before,
                    block_number,
                    transaction_replay,
                    block,
                    gas_price,
                });
            }
        }
        eyre::bail!(
            "fork endpoint changed while its identity and block context were being resolved"
        );
    }

    pub(crate) async fn fork_context_matches(
        &self,
        eth_rpc_url: &str,
        expected: ForkEndpointIdentity,
        block_number: u64,
        block_hash: B256,
    ) -> Result<bool> {
        let provider = self.fork_provider(eth_rpc_url)?;
        let mut node_info_probe = AnvilNodeInfoProbe::new(expected.is_authoritative());
        for _ in 0..3 {
            let before =
                self.resolved_fork_endpoint_identity(&provider, &mut node_info_probe).await?;
            let block = provider
                .get_block(BlockNumberOrTag::Number(block_number).into())
                .await
                .wrap_err("failed to confirm fork block context")?;
            let after =
                self.resolved_fork_endpoint_identity(&provider, &mut node_info_probe).await?;
            if before != after {
                continue;
            }
            if before.source_chain_id != expected.source_chain_id {
                eyre::bail!(
                    "fork endpoints must use the same chain ID: expected {}, got {} from {}",
                    expected.source_chain_id,
                    before.source_chain_id,
                    redact_url(eth_rpc_url)
                );
            }
            return Ok(
                before == expected && block.is_some_and(|block| block.header.hash == block_hash)
            );
        }
        Ok(false)
    }

    pub(crate) async fn fork_urls_match_context(
        &self,
        fork_urls: &[String],
        expected: ForkEndpointIdentity,
        block_number: u64,
        block_hash: B256,
    ) -> Result<bool> {
        for eth_rpc_url in Self::fork_urls_requiring_revalidation(fork_urls, expected) {
            if !self.fork_context_matches(eth_rpc_url, expected, block_number, block_hash).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    const fn fork_urls_requiring_revalidation(
        fork_urls: &[String],
        endpoint_identity: ForkEndpointIdentity,
    ) -> &[String] {
        if endpoint_identity.is_authoritative() || fork_urls.len() > 1 { fork_urls } else { &[] }
    }

    pub(crate) fn has_explicit_network_selection(&self) -> bool {
        let effective_network =
            self.networks.resolved_network().unwrap_or(NetworkVariant::Ethereum);
        self.networks.has_network_selection()
            && self.chain_id_network_base.is_none()
            && self.inferred_fork_network != Some(effective_network)
    }

    const fn requires_primary_fork_revalidation(
        &self,
        endpoint_identity: ForkEndpointIdentity,
    ) -> bool {
        endpoint_identity.is_authoritative() || self.fork_urls.len() > 1
    }

    /// Configures everything related to forking based on the passed `eth_rpc_url`:
    ///  - returning a tuple of a [ForkedDatabase] and [ClientForkConfig] which can be used to build
    ///    a [ClientFork] to fork from.
    ///  - modifying some parameters of the passed `env`
    ///  - mutating some members of `self`
    pub async fn setup_fork_db_config(
        &mut self,
        eth_rpc_url: String,
        evm_env: &mut EvmEnv,
        fees: &FeeManager,
    ) -> Result<(ForkedDatabase<AnyNetwork>, ClientForkConfig)> {
        let (db, config, replay) =
            self.setup_fork_db_config_with_replay(eth_rpc_url, evm_env, fees).await?;
        eyre::ensure!(replay.is_none(), "transaction-hash fork replay requires full node startup");
        Ok((db, config))
    }

    pub(crate) async fn setup_fork_db_config_with_replay(
        &mut self,
        eth_rpc_url: String,
        evm_env: &mut EvmEnv,
        fees: &FeeManager,
    ) -> Result<(ForkedDatabase<AnyNetwork>, ClientForkConfig, Option<ForkTransactionReplay>)> {
        debug!(target: "node", eth_rpc_url=%redact_url(&eth_rpc_url), "setting up fork db");
        if self.fork_chain_id.is_some() {
            eyre::ensure!(
                self.fork_urls.len() == 1,
                "multiple fork URLs cannot be validated with --fork-chain-id; remove \
                 --fork-chain-id to validate every endpoint"
            );
        }
        let fork_overrides = *self.fork_overrides.get_or_insert(ForkOverrides {
            gas_limit: self.gas_limit,
            gas_price: self.gas_price,
            base_fee: self.base_fee,
        });

        // Always bootstrap with the primary URL only to avoid race conditions
        // where discovery calls (get_chain_id, find_latest_fork_block, get_block)
        // hit different endpoints that may be at different chain tips.
        let provider = Arc::new(self.fork_provider(&eth_rpc_url)?);

        // Resolve identity, block, and fee data as one stable snapshot. An upstream Anvil can
        // reset between any two RPC calls, so verify the endpoint identity on both sides.
        let StableForkSnapshot {
            endpoint_identity: fork_identity,
            block_number: fork_block_number,
            transaction_replay: fork_transaction_replay,
            block,
            gas_price,
        } = self.stable_fork_snapshot(&provider, fork_overrides).await?;
        self.fork_endpoint_is_anvil = fork_identity.is_authoritative();

        let target_network = fork_identity.network.unwrap_or(NetworkVariant::Ethereum);
        let target_profile = fork_identity.network_profile.unwrap_or_default();
        if self.inferred_fork_network.is_some()
            && !self.networks.supports_fork_source(&target_profile)
        {
            eyre::bail!(
                "cannot reset Anvil across network families ({} -> {}); start a new instance \
                 with matching network configuration",
                self.networks.execution_profile_name(),
                target_profile.execution_profile_name()
            );
        }
        let source_chain_id = fork_identity.source_chain_id;
        self.fork_source_chain_id = Some(source_chain_id);
        self.fork_execution_chain_id = Some(fork_identity.execution_chain_id);
        if !self.has_explicit_network_selection() {
            self.networks = self.networks.with_rpc_profile(target_profile);
            self.inferred_fork_network = Some(target_network);
            self.chain_id_network_base = None;
        }

        let block = if let Some(block) = block {
            block
        } else {
            if let Ok(latest_block) = provider.get_block_number().await {
                let mut message = format!(
                    "Failed to get block for block number: {fork_block_number}\n\
latest block number: {latest_block}"
                );
                // If the `eth_getBlockByNumber` call succeeds, but returns null instead of
                // the block, and the block number is less than equal the latest block, then
                // the user is forking from a non-archive node with an older block number.
                if fork_block_number <= latest_block {
                    message.push_str(&format!("\n{NON_ARCHIVE_NODE_WARNING}"));
                }
                eyre::bail!("{message}");
            }
            eyre::bail!("failed to get block for block number: {fork_block_number}");
        };

        if let Some(replay) = &fork_transaction_replay {
            let source_header = replay.source_block.header();
            eyre::ensure!(
                block.header.hash == source_header.parent_hash,
                "fork transaction block {} at {} has parent {}, but fetched fork block at {} has \
                 hash {}",
                source_header.hash,
                source_header.number,
                source_header.parent_hash,
                block.header.number,
                block.header.hash
            );
            eyre::ensure!(
                block.header.number.checked_add(1) == Some(source_header.number),
                "fork transaction block {} has number {}, but fetched parent {} has number {}",
                source_header.hash,
                source_header.number,
                block.header.hash,
                block.header.number
            );
        }

        let gas_limit = self.fork_gas_limit_with_override(&block, fork_overrides.gas_limit);
        self.gas_limit = Some(gas_limit);

        // Cache identity must describe the remote fork block, not local execution overrides that
        // can change after mining (for example, the locally advanced base fee).
        let cache_block_env: BlockEnv = block_env_from_header(&block.header);

        evm_env.block_env = BlockEnv {
            gas_limit,
            // Preserve configured local overrides while replacing fork-derived block values.
            beneficiary: evm_env.block_env.beneficiary,
            basefee: fork_overrides
                .base_fee
                .or_else(|| block.header.base_fee_per_gas())
                .unwrap_or_default(),
            ..block_env_from_header(&block.header)
        };

        let override_chain_id = self.chain_id;
        let execution_chain_id = override_chain_id.unwrap_or(fork_identity.execution_chain_id);
        if override_chain_id.is_none() {
            // Sign locally produced transactions for the chain ID exposed by the endpoint
            // without turning the inferred value into an explicit execution override.
            self.update_wallet_chain_id(fork_identity.execution_chain_id);
        }
        evm_env.cfg_env.chain_id = execution_chain_id;

        // Resolve the fork block's hardfork without materializing it into `self.hardfork`.
        // That field represents the user's explicit override; keeping inference on the fork
        // config lets a later reset re-resolve timestamp-based activations.
        let effective_network =
            self.networks.resolved_network().unwrap_or(NetworkVariant::Ethereum);
        let endpoint_matches_execution = fork_identity.network == Some(effective_network);
        let source_hardfork = fork_identity.hardfork.or_else(|| {
            FoundryHardfork::from_chain_and_timestamp(source_chain_id, block.header.timestamp())
        });
        let inferred_hardfork = source_hardfork.filter(|hardfork| {
            endpoint_matches_execution
                && hardfork.namespace() == effective_network.hardfork_namespace()
        });
        let source_may_omit_blob_fields = source_hardfork
            .map_or(self.hardfork.is_some(), |hardfork| SpecId::from(hardfork) < SpecId::CANCUN);
        let fork_hardfork = self.hardfork.or(inferred_hardfork);
        let effective_hardfork = fork_hardfork.unwrap_or_else(|| self.get_hardfork());
        let effective_spec = SpecId::from(effective_hardfork);
        evm_env.cfg_env.set_spec_and_mainnet_gas_params(effective_spec);
        fees.set_execution_rules(
            effective_spec,
            self.networks.base_fee_params(block.header.timestamp()),
            self.networks.is_tempo().then(|| TempoHardfork::from(effective_hardfork)),
        );
        #[cfg(feature = "optimism")]
        if self.networks.is_optimism() {
            fees.set_optimism_base_fee_rules(block.header.extra_data());
        }

        // if not set explicitly we use the base fee of the latest block
        self.base_fee = fork_overrides.base_fee.or_else(|| block.header.base_fee_per_gas());
        if let Some(base_fee) = fork_overrides.base_fee {
            fees.set_base_fee(base_fee);
        } else if let Some(base_fee) = block.header.base_fee_per_gas() {
            // This is the base fee of the current block, but we need the base fee of the next
            // block.
            fees.set_base_fee(base_fee);
            let next_block_base_fee = fees.get_next_block_base_fee_from_header(&block.header);
            fees.set_base_fee(next_block_base_fee);
        } else {
            fees.set_base_fee(self.get_base_fee());
        }

        // Blob rules and fee state belong to the selected fork context even when the target block
        // predates Cancun. Always replace both so a reset cannot retain the previous fork's
        // schedule or excess gas.
        let blob_params = get_blob_params(source_chain_id, block.header.timestamp());
        fees.set_blob_params(blob_params);
        let blob_update_fraction = blob_params.update_fraction as u64;
        let blob_excess_gas = block.header.excess_blob_gas().or_else(|| {
            // Pre-Cancun headers, Polygon Bor headers, and Arbitrum Nitro headers omit the blob
            // fields. REVM still requires a valid blob environment when executing with the Cancun
            // spec; zero is the neutral excess-gas value. On Nitro this makes `BLOBBASEFEE` return
            // `1`, although Nitro rejects the opcode; matching that requires Arbitrum-specific EVM
            // handling.
            (effective_spec >= SpecId::CANCUN
                && ((source_may_omit_blob_fields && block.header.blob_gas_used().is_none())
                    || Chain::from_id(source_chain_id).is_polygon()
                    || Chain::from_id(source_chain_id).is_arbitrum()))
            .then_some(0)
        });
        evm_env.block_env.blob_excess_gas_and_price =
            blob_excess_gas.map(|excess| BlobExcessGasAndPrice::new(excess, blob_update_fraction));
        let next_block_blob_excess_gas = blob_excess_gas.map_or(0, |excess| {
            self.networks.next_block_blob_excess_gas(
                blob_params,
                excess,
                block.header.blob_gas_used().unwrap_or_default(),
                block.header.base_fee_per_gas().unwrap_or_default(),
            )
        });
        fees.set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
            next_block_blob_excess_gas,
            blob_update_fraction,
        ));

        // Use the gas price captured in the stable endpoint snapshot.
        self.gas_price = Some(gas_price);
        fees.set_gas_price(gas_price);

        let block_hash = block.header.hash;

        // Apply changes such as difficulty -> prevrandao for the remote source chain.
        apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _, _>(
            evm_env,
            &block,
            source_chain_id,
            self.networks,
        );

        for mirror_url in self.fork_urls.iter().skip(1) {
            if !self
                .fork_context_matches(mirror_url, fork_identity, fork_block_number, block_hash)
                .await?
            {
                eyre::bail!(
                    "fork fallback endpoint `{}` does not expose the primary endpoint's execution \
                     and block context",
                    redact_url(mirror_url)
                );
            }
        }
        if self.requires_primary_fork_revalidation(fork_identity)
            && !self
                .fork_context_matches(&eth_rpc_url, fork_identity, fork_block_number, block_hash)
                .await?
        {
            eyre::bail!("primary fork endpoint changed while its context was being validated");
        }

        let source_id = fork_source_id(&self.fork_urls, &self.fork_headers);
        let meta = BlockchainDbMeta::new(cache_block_env, eth_rpc_url.clone())
            .with_fork_identity(block_hash, source_id);
        let cache_path =
            self.block_cache_path_for_rpc(source_chain_id, fork_block_number, &eth_rpc_url);
        let block_chain_db = BlockchainDb::new(meta, cache_path);

        // After bootstrap, rebuild the provider with round-robin if multiple URLs are
        // configured. This ensures bootstrap used only the primary endpoint for consistency,
        // while ongoing requests are distributed across all endpoints.
        let provider = if self.fork_urls.len() > 1 {
            let urls = self.fork_urls.iter().map(|url| redact_url(url)).collect::<Vec<_>>();
            debug!(target: "node", ?urls, "using multi-endpoint round-robin provider");
            Arc::new(
                ProviderBuilder::new(&eth_rpc_url)
                    .timeout(self.fork_request_timeout)
                    .initial_backoff(self.fork_retry_backoff.as_millis() as u64)
                    .compute_units_per_second(self.compute_units_per_second)
                    .max_retry(self.fork_request_retries)
                    .headers(self.fork_headers.clone())
                    .build_fallback(self.fork_urls.clone())
                    .wrap_err("failed to establish round-robin provider to fork urls")?,
            )
        } else {
            provider
        };

        // This will spawn the background thread that will use the provider to fetch
        // blockchain data from the other client
        let anchor = ForkBlock::with_rpc_number(
            evm_env.block_env.number.saturating_to(),
            fork_block_number,
            block_hash,
        );
        let (backend, handler) =
            SharedBackend::new_with_anchor(Arc::clone(&provider), block_chain_db.clone(), anchor)?;
        tokio::spawn(handler);

        let config = ClientForkConfig {
            fork_urls: self.fork_urls.clone(),
            block_number: fork_block_number,
            block_hash,
            transaction_hash: self.fork_choice.and_then(|fc| fc.transaction_hash()),
            provider,
            chain_id: source_chain_id,
            execution_chain_id,
            override_chain_id,
            fork_chain_id: self.fork_chain_id.map(|chain_id| chain_id.to()),
            hardfork: Some(effective_hardfork),
            endpoint_identity: fork_identity,
            timestamp: block.header.timestamp(),
            base_fee: block.header.base_fee_per_gas().map(|g| g as u128),
            timeout: self.fork_request_timeout,
            retries: self.fork_request_retries,
            backoff: self.fork_retry_backoff,
            compute_units_per_second: self.compute_units_per_second,
            headers: self.fork_headers.clone(),
            total_difficulty: block.header.total_difficulty.unwrap_or_default(),
            blob_gas_used: block.header.blob_gas_used().map(|g| g as u128),
            blob_excess_gas_and_price: evm_env.block_env.blob_excess_gas_and_price,
        };

        debug!(target: "node", fork_number=config.block_number, fork_hash=%config.block_hash, "set up fork db");

        let mut db = ForkedDatabase::new(backend, block_chain_db);

        // need to insert the forked block's hash
        db.insert_block_hash(U256::from(config.block_number), config.block_hash);

        Ok((db, config, fork_transaction_replay))
    }

    /// we only use the gas limit value of the block if it is non-zero and the block gas
    /// limit is enabled, since there are networks where this is not used and is always
    /// `0x0` which would inevitably result in `OutOfGas` errors as soon as the evm is about to record gas, See also <https://github.com/foundry-rs/foundry/issues/3247>
    fn fork_gas_limit_with_override<B: BlockResponse<Header: BlockHeader>>(
        &self,
        block: &B,
        gas_limit: Option<u64>,
    ) -> u64 {
        if !self.disable_block_gas_limit {
            if let Some(gas_limit) = gas_limit {
                return gas_limit;
            } else if block.header().gas_limit() > 0 {
                return block.header().gas_limit();
            }
        }

        u64::MAX
    }

    /// Restores user-provided gas settings after leaving fork mode.
    pub(crate) const fn restore_fork_overrides(&mut self) {
        if let Some(overrides) = self.fork_overrides {
            self.gas_limit = overrides.gas_limit;
            self.gas_price = overrides.gas_price;
            self.base_fee = overrides.base_fee;
        }
    }

    /// Returns the gas limit for a non forked anvil instance
    ///
    /// Checks the config for the `disable_block_gas_limit` flag
    pub(crate) fn gas_limit(&self) -> u64 {
        if self.disable_block_gas_limit {
            return u64::MAX;
        }

        self.gas_limit.unwrap_or(DEFAULT_GAS_LIMIT)
    }
}

pub(crate) const fn tempo_default_base_fee(hardfork: TempoHardfork) -> u64 {
    if hardfork.is_t1() { TEMPO_T1_BASE_FEE } else { TEMPO_T0_BASE_FEE }
}

/// If the fork choice is a block number, simply return it with an empty list of transactions.
/// If the fork choice is a transaction hash, determine the block that the transaction was mined in,
/// and return the block number before the fork block along with all transactions in the fork block
/// that are before (and including) the fork transaction.
async fn derive_block_and_replay(
    fork_choice: &ForkChoice,
    provider: &Arc<RetryProvider>,
) -> eyre::Result<(BlockNumber, Option<ForkTransactionReplay>)> {
    match fork_choice {
        ForkChoice::Block(block_number) => {
            let block_number = *block_number;
            if block_number >= 0 {
                return Ok((block_number as u64, None));
            }
            // subtract from latest block number
            let latest = provider.get_block_number().await?;

            Ok((block_number.saturating_add(latest as i128) as u64, None))
        }
        ForkChoice::Transaction(transaction_hash) => {
            // Determine the block that this transaction was mined in
            let transaction = provider
                .get_transaction_by_hash(transaction_hash.0.into())
                .await?
                .ok_or_else(|| eyre::eyre!("fork transaction {transaction_hash} was not found"))?;
            let transaction_block_number = transaction.block_number().ok_or_else(|| {
                eyre::eyre!("fork transaction {transaction_hash} is not mined (no block number)")
            })?;
            let transaction_block_hash = transaction.block_hash().ok_or_else(|| {
                eyre::eyre!("fork transaction {transaction_hash} is not mined (no block hash)")
            })?;

            // Get the block pertaining to the fork transaction.
            let transaction_block =
                provider.get_block_by_hash(transaction_block_hash).full().await?.ok_or_else(
                    || {
                        eyre::eyre!(
                            "failed to get fork block {transaction_block_hash} for transaction \
                         {transaction_hash}"
                        )
                    },
                )?;
            let replay = validate_fork_transaction_replay(
                *transaction_hash,
                &transaction,
                transaction_block,
            )?;
            Ok((transaction_block_number.saturating_sub(1), Some(replay)))
        }
    }
}

fn validate_fork_transaction_replay(
    transaction_hash: TxHash,
    transaction: &alloy_network::AnyRpcTransaction,
    source_block: AnyRpcBlock,
) -> eyre::Result<ForkTransactionReplay> {
    let source_hash = source_block.header.hash;
    let source_number = source_block.header.number;
    let transaction_block_hash = transaction.block_hash().ok_or_else(|| {
        eyre::eyre!("fork transaction {transaction_hash} is not mined (no block hash)")
    })?;
    let transaction_block_number = transaction.block_number().ok_or_else(|| {
        eyre::eyre!("fork transaction {transaction_hash} is not mined (no block number)")
    })?;

    eyre::ensure!(
        source_hash == transaction_block_hash,
        "fork transaction {transaction_hash} reports block {transaction_block_hash}, but fetched \
         block hash is {source_hash}"
    );
    eyre::ensure!(
        source_number == transaction_block_number,
        "fork transaction {transaction_hash} reports block number {transaction_block_number}, but \
         fetched block {source_hash} has number {source_number}"
    );
    eyre::ensure!(
        source_number > 0,
        "fork transaction {transaction_hash} is in genesis block {source_hash}, which has no parent"
    );

    let transactions = source_block.transactions.as_transactions().ok_or_else(|| {
        eyre::eyre!("fork block {source_hash} at {source_number} did not include full transactions")
    })?;
    let mut matches =
        transactions.iter().enumerate().filter(|(_, tx)| tx.tx_hash() == transaction_hash);
    let target_index = matches.next().map(|(index, _)| index).ok_or_else(|| {
        eyre::eyre!(
            "fork transaction {transaction_hash} is absent from block {source_hash} at \
             {source_number}"
        )
    })?;
    eyre::ensure!(
        matches.next().is_none(),
        "fork transaction {transaction_hash} occurs more than once in block {source_hash} at \
         {source_number}"
    );
    if let Some(reported_index) = transaction.transaction_index() {
        eyre::ensure!(
            reported_index == target_index as u64,
            "fork transaction {transaction_hash} reports index {reported_index}, but occurs at \
             index {target_index} in block {source_hash}"
        );
    }

    Ok(ForkTransactionReplay { source_block, target_index })
}

/// Fork delimiter used to specify which block or transaction to fork from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkChoice {
    /// Block number to fork from.
    ///
    /// If negative, the given value is subtracted from the `latest` block number.
    Block(i128),
    /// Transaction hash to fork from.
    Transaction(TxHash),
}

impl ForkChoice {
    /// Returns the block number to fork from
    pub const fn block_number(&self) -> Option<i128> {
        match self {
            Self::Block(block_number) => Some(*block_number),
            Self::Transaction(_) => None,
        }
    }

    /// Returns the transaction hash to fork from
    pub const fn transaction_hash(&self) -> Option<TxHash> {
        match self {
            Self::Block(_) => None,
            Self::Transaction(transaction_hash) => Some(*transaction_hash),
        }
    }
}

/// Convert a transaction hash into a ForkChoice
impl From<TxHash> for ForkChoice {
    fn from(tx_hash: TxHash) -> Self {
        Self::Transaction(tx_hash)
    }
}

/// Convert a decimal block number into a ForkChoice
impl From<u64> for ForkChoice {
    fn from(block: u64) -> Self {
        Self::Block(block as i128)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PruneStateHistoryConfig {
    pub enabled: bool,
    pub max_memory_history: Option<usize>,
}

impl PruneStateHistoryConfig {
    /// Returns `true` if writing state history is supported
    pub const fn is_state_history_supported(&self) -> bool {
        if !self.enabled {
            return true;
        }

        match self.max_memory_history {
            Some(limit) => limit > 0,
            None => false,
        }
    }

    /// Returns true if this setting was enabled.
    pub const fn is_config_enabled(&self) -> bool {
        self.enabled
    }

    pub fn from_args(val: Option<Option<usize>>) -> Self {
        val.map(|max_memory_history| Self {
            enabled: true,
            max_memory_history: max_memory_history.filter(|limit| *limit > 0),
        })
        .unwrap_or_default()
    }
}

/// Can create dev accounts
#[derive(Clone, Debug)]
pub struct AccountGenerator {
    chain_id: u64,
    amount: usize,
    phrase: String,
    derivation_path: Option<String>,
}

impl AccountGenerator {
    pub fn new(amount: usize) -> Self {
        Self {
            chain_id: CHAIN_ID,
            amount,
            phrase: Mnemonic::<English>::new(&mut thread_rng()).to_phrase(),
            derivation_path: None,
        }
    }

    #[must_use]
    pub fn phrase(mut self, phrase: impl Into<String>) -> Self {
        self.phrase = phrase.into();
        self
    }

    fn get_phrase(&self) -> &str {
        &self.phrase
    }

    #[must_use]
    pub fn chain_id(mut self, chain_id: impl Into<u64>) -> Self {
        self.chain_id = chain_id.into();
        self
    }

    #[must_use]
    pub fn derivation_path(mut self, derivation_path: impl Into<String>) -> Self {
        let mut derivation_path = derivation_path.into();
        if !derivation_path.ends_with('/') {
            derivation_path.push('/');
        }
        self.derivation_path = Some(derivation_path);
        self
    }

    fn get_derivation_path(&self) -> &str {
        self.derivation_path.as_deref().unwrap_or("m/44'/60'/0'/0/")
    }
}

impl AccountGenerator {
    pub fn generate(&self) -> eyre::Result<Vec<PrivateKeySigner>> {
        let builder = MnemonicBuilder::<English>::default().phrase(self.phrase.as_str());

        // use the derivation path
        let derivation_path = self.get_derivation_path();
        foundry_common::wallet::validate_bip32_path(derivation_path).map_err(|e| eyre::eyre!(e))?;

        let mut wallets = Vec::with_capacity(self.amount);
        for idx in 0..self.amount {
            let idx = u32::try_from(idx).map_err(|_| eyre::eyre!("account index overflows u32"))?;
            let full_path = foundry_common::wallet::derive_key_path_checked(derivation_path, idx)
                .map_err(|e| eyre::eyre!(e))?;
            let builder = builder.clone().derivation_path(full_path)?;
            let wallet = builder.build()?.with_chain_id(Some(self.chain_id));
            wallets.push(wallet)
        }
        Ok(wallets)
    }
}

/// Returns the path to anvil dir `~/.foundry/anvil`
pub fn anvil_dir() -> Option<PathBuf> {
    Config::foundry_dir().map(|p| p.join("anvil"))
}

/// Returns the root path to anvil's temporary storage `~/.foundry/anvil/`
pub fn anvil_tmp_dir() -> Option<PathBuf> {
    anvil_dir().map(|p| p.join("tmp"))
}

/// Finds the latest appropriate block to fork
///
/// This fetches the "latest" block and checks whether the `Block` is fully populated (`hash` field
/// is present). This prevents edge cases where anvil forks the "latest" block but `eth_getBlockByNumber` still returns a pending block, <https://github.com/foundry-rs/foundry/issues/2036>
async fn find_latest_fork_block<P: Provider<AnyNetwork>>(
    provider: P,
) -> Result<u64, TransportError> {
    let mut num = provider.get_block_number().await?;

    // walk back from the head of the chain, but at most 2 blocks, which should be more than enough
    // leeway
    for _ in 0..2 {
        if let Some(block) = provider.get_block(num.into()).await?
            && !block.header.hash.is_zero()
        {
            break;
        }
        // block not actually finalized, so we try the block before
        num = num.saturating_sub(1)
    }

    Ok(num)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "optimism")]
    use foundry_evm::hardfork::OpHardfork;
    #[cfg(feature = "base")]
    use foundry_evm::hardforks::BaseUpgrade;
    use foundry_evm::{hardfork::EthereumHardfork, hardforks::latest_active_tempo_hardfork};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_output_redacts_endpoint_credentials() {
        let (_api, source) = crate::spawn(NodeConfig::test()).await;
        let fork_url = source.http_endpoint().replacen("http://", "http://user:password@", 1)
            + "/?api_key=secret";
        let mut config = NodeConfig::test().with_eth_rpc_url(Some(fork_url.clone()));
        let (api, _handle) = crate::spawn(config.clone()).await;
        config.fork_urls.push("https://mirror.example/private-api-key?token=secret".to_string());

        let fork = api.backend.get_fork().unwrap();
        let output = config.as_string(Some(&fork));
        let temp = tempfile::tempdir().unwrap();
        let config_out = temp.path().join("config.json");
        config.config_out = Some(config_out.clone());
        config.print(Some(&fork)).unwrap();
        let json = serde_json::from_slice::<Value>(&std::fs::read(config_out).unwrap()).unwrap();

        assert!(output.contains(&redact_url(&fork_url)));
        assert!(output.contains("https://mirror.example/"));
        assert!(!output.contains("user"));
        assert!(!output.contains("password"));
        assert!(!output.contains("private-api-key"));
        assert!(!output.contains("secret"));
        assert_eq!(json["endpoint"], redact_url(&fork_url));
        assert!(!json.to_string().contains("password"));
        assert!(!json.to_string().contains("secret"));
    }

    #[test]
    fn fork_source_identity_includes_all_urls_and_headers() {
        let urls = ["http://primary".to_string(), "http://fallback".to_string()];
        let headers = ["Authorization: secret".to_string()];
        let identity = fork_source_id(&urls, &headers);

        assert_ne!(identity, fork_source_id(&urls[..1], &headers));
        assert_ne!(identity, fork_source_id(&urls, &[]));
        assert_ne!(identity, fork_source_id(&[urls[1].clone(), urls[0].clone()], &headers));
    }

    #[test]
    fn test_prune_history() {
        let config = PruneStateHistoryConfig::default();
        assert!(config.is_state_history_supported());
        let config = PruneStateHistoryConfig::from_args(Some(None));
        assert!(!config.is_state_history_supported());
        let config = PruneStateHistoryConfig::from_args(Some(Some(0)));
        assert!(config.is_config_enabled());
        assert!(!config.is_state_history_supported());
        let config = PruneStateHistoryConfig::from_args(Some(Some(10)));
        assert!(config.is_state_history_supported());
    }

    #[test]
    fn fork_cache_path_can_use_source_chain() {
        let rpc_url = "http://localhost:8545";
        let mut config = NodeConfig::test()
            .with_eth_rpc_url(Some(rpc_url.to_string()))
            .with_chain_id(Some(1u64));
        let block = 42;
        config.fork_source_chain_id = Some(143);
        let expected = Config::foundry_block_cache_file(143, block).map(|path| {
            path.with_file_name(format!("storage-{}.json", hex::encode(keccak256(rpc_url))))
        });

        assert_eq!(config.block_cache_path(block), expected);
        assert_ne!(
            config.block_cache_path_for_rpc(143, block, rpc_url),
            config.block_cache_path_for_rpc(143, block, "http://localhost:8546")
        );
    }

    #[test]
    fn fork_execution_and_source_chain_ids_remain_distinct() {
        let mut config = NodeConfig::test();
        config.fork_execution_chain_id = Some(1);
        config.fork_source_chain_id = Some(143);

        assert_eq!(config.get_chain_id(), 1);
        assert_eq!(config.protocol_chain_id(), 143);
    }

    #[test]
    fn fork_chain_id_is_only_an_offline_discovery_hint() {
        let mut config = NodeConfig::test()
            .with_chain_id(Some(31_337u64))
            .with_fork_chain_id(Some(U256::from(143)));

        assert_eq!(config.protocol_chain_id(), 31_337);

        config.fork_source_chain_id = Some(143);
        assert_eq!(config.protocol_chain_id(), 143);

        config.fork_source_chain_id = None;
        assert_eq!(config.protocol_chain_id(), 31_337);
    }

    #[test]
    fn fork_endpoint_revalidation_requires_authority_or_fallbacks() {
        let anonymous = ForkEndpointIdentity {
            execution_chain_id: 1,
            source_chain_id: 1,
            network: Some(NetworkVariant::Ethereum),
            network_profile: Some(NetworkConfigs::default()),
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };
        let mut config =
            NodeConfig::test().with_eth_rpc_url(Some("http://localhost:8545".to_string()));

        assert!(!anonymous.is_authoritative());
        assert!(!config.requires_primary_fork_revalidation(anonymous));

        config.fork_urls.push("http://localhost:8546".to_string());
        assert!(config.requires_primary_fork_revalidation(anonymous));
        assert_eq!(
            NodeConfig::fork_urls_requiring_revalidation(&config.fork_urls, anonymous),
            config.fork_urls
        );

        config.fork_urls.pop();
        let authoritative =
            ForkEndpointIdentity { hardfork: Some(EthereumHardfork::Prague.into()), ..anonymous };
        assert!(authoritative.is_authoritative());
        assert!(config.requires_primary_fork_revalidation(authoritative));
        assert_eq!(
            NodeConfig::fork_urls_requiring_revalidation(&config.fork_urls, authoritative),
            config.fork_urls
        );
    }

    #[tokio::test]
    async fn fork_authoritative_identity_keeps_node_info_probe_strict() {
        let (_api, origin) =
            crate::spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Mainnet as u64))).await;
        let fork_url = foundry_test_utils::rpc::spawn_rpc_proxy_rejecting_method_after(
            origin.http_endpoint(),
            "anvil_nodeInfo",
            0,
        )
        .await;
        let expected = ForkEndpointIdentity {
            execution_chain_id: NamedChain::Mainnet as u64,
            source_chain_id: NamedChain::Mainnet as u64,
            network: Some(NetworkVariant::Ethereum),
            network_profile: Some(NetworkConfigs::default()),
            hardfork: Some(EthereumHardfork::Prague.into()),
            instance_id: Some(B256::with_last_byte(1)),
            source_fork_block_number: None,
            source_fork_block_hash: None,
        };

        let error = NodeConfig::test()
            .fork_context_matches(&fork_url, expected, 0, B256::ZERO)
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("failed to determine network family from fork endpoint"),
            "{error}"
        );
    }

    #[cfg(feature = "optimism")]
    #[test]
    fn set_chain_id_updates_network_config() {
        let mut config = NodeConfig::test();
        config.set_chain_id(Some(10u64));

        assert!(config.networks.is_optimism());
    }

    #[test]
    fn chain_id_network_inference_is_replaceable_and_clearable() {
        let mut config = NodeConfig::test();
        config.set_chain_id(Some(4217u64));
        assert!(config.networks.is_tempo());

        config.set_chain_id(Some(NamedChain::Celo as u64));
        assert!(config.networks.is_celo());
        assert!(!config.networks.is_tempo());

        config.set_chain_id(Some(1u64));
        assert!(!config.networks.has_network_selection());

        config.set_chain_id(Some(4217u64));
        config.set_chain_id(None::<u64>);
        assert!(!config.networks.has_network_selection());
    }

    #[test]
    fn chain_id_preserves_explicit_network_selection() {
        let mut config = NodeConfig::test_tempo();
        config.set_chain_id(Some(NamedChain::Celo as u64));

        assert!(config.networks.is_tempo());
        assert!(!config.networks.is_celo());
    }

    #[test]
    fn get_hardfork_on_tempo_never_returns_non_tempo_variant() {
        // Post-Shanghai timestamp on Ethereum mainnet.
        let shanghai_ts = 1_681_338_455u64;

        let config = NodeConfig::test_tempo()
            .with_chain_id(Some(1u64))
            .with_genesis_timestamp(Some(shanghai_ts));

        assert!(config.networks.is_tempo());
        assert!(matches!(config.get_hardfork(), FoundryHardfork::Tempo(_)));
    }

    #[test]
    fn get_hardfork_on_ethereum_uses_genesis_timestamp() {
        let timestamp = EthereumHardfork::Shanghai.mainnet_activation_timestamp().unwrap();
        let config =
            NodeConfig::test().with_chain_id(Some(1u64)).with_genesis_timestamp(Some(timestamp));

        assert_eq!(config.get_hardfork(), FoundryHardfork::Ethereum(EthereumHardfork::Shanghai));
    }

    #[test]
    #[cfg(feature = "optimism")]
    fn get_hardfork_on_optimism_uses_genesis_timestamp() {
        // OP Mainnet Canyon activation timestamp.
        let timestamp = 1_704_992_401u64;
        let config = NodeConfig::test()
            .with_optimism()
            .with_chain_id(Some(10u64))
            .with_genesis_timestamp(Some(timestamp));

        assert_eq!(config.get_hardfork(), FoundryHardfork::Optimism(OpHardfork::Canyon));
    }

    #[test]
    fn get_hardfork_on_local_tempo_defaults_to_latest_active() {
        let config = NodeConfig::test_tempo();

        assert_eq!(config.get_hardfork(), FoundryHardfork::Tempo(latest_active_tempo_hardfork()));
    }

    #[test]
    #[cfg(feature = "base")]
    fn test_base_config_uses_base_network_and_chain_id() {
        let config = NodeConfig::test_base();

        assert!(config.networks.is_base());
        assert_eq!(config.get_chain_id(), NamedChain::Base as u64);
    }

    #[test]
    #[cfg(feature = "base")]
    fn get_hardfork_on_base_fork_uses_source_chain_timestamp_mapping() {
        let mut config = NodeConfig::test_base()
            .with_chain_id(Some(1u64))
            .with_genesis_timestamp(Some(u64::MAX));
        config.fork_source_chain_id = Some(NamedChain::Base as u64);

        assert_eq!(config.get_chain_id(), 1);
        assert_eq!(config.get_hardfork(), FoundryHardfork::Base(BaseUpgrade::Beryl));
    }

    #[test]
    #[cfg(feature = "monad")]
    fn get_hardfork_on_monad_fork_uses_source_chain_timestamp_mapping() {
        let mut config = NodeConfig::test_monad()
            .with_chain_id(Some(1u64))
            .with_genesis_timestamp(Some(1_763_648_999u64));
        config.fork_source_chain_id = Some(143);

        assert_eq!(config.get_chain_id(), 1);
        assert_eq!(
            config.get_hardfork(),
            FoundryHardfork::Monad(foundry_evm::hardfork::MonadHardfork::MonadEight)
        );
    }

    #[test]
    fn account_generator_rejects_harden_bit_overflow_path() {
        let err = AccountGenerator::new(1)
            .phrase("test test test test test test test test test test test junk")
            .derivation_path("m/44'/60'/0'/0/2147483648'")
            .generate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("harden bit"), "{err}");

        assert!(
            AccountGenerator::new(1)
                .phrase("test test test test test test test test test test test junk")
                .derivation_path("m/44'/60'/0'/0")
                .generate()
                .is_ok()
        );
    }
}
