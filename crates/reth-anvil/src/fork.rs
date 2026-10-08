//! Forking: chain data and state below the fork block come from a remote endpoint.
//!
//! The local database starts at the fork block, which stands in for genesis: its header is the
//! remote header, so the next local block links to the remote chain. Blocks below the fork block
//! are fetched from the remote endpoint on demand. State reads check whether the local chain has
//! written the account or slot since the fork, through reth's history index and the in-memory
//! blocks; everything else is read from the remote endpoint at the fork block, through foundry's
//! cached fork database.

use crate::{
    config::NodeConfig,
    state_dump::{
        SerializableBlock, SerializableState, SerializableTransaction, SnapshotAccount,
        StateSnapshot, json_convert,
    },
    types::ForkChoice,
};
use alloy_consensus::{
    BlockHeader, Header, TxReceipt, TxType,
    transaction::{SignerRecoverable, TransactionMeta, TxHashRef},
};
use alloy_eips::{BlockHashOrNumber, BlockId};
use alloy_network::{AnyNetwork, AnyRpcBlock, AnyRpcTransaction, AnyTransactionReceipt, Network};
use alloy_primitives::{Address, B256, Bytes, StorageKey, StorageValue, TxNumber, U256, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::anvil::{Metadata, NodeInfo};
use eyre::{Result, WrapErr};
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use foundry_config::Config;
use foundry_evm_core::{backend::account_fetch_policy_for_source, utils::block_env_from_header};
use foundry_evm_networks::NetworkConfigs;
use foundry_fork_db::{
    AccountFetchPolicy, BlockchainDb, ForkBlock as ForkAnchor, SharedBackend,
    backend::BlockingMode, cache::BlockchainDbMeta,
};
use foundry_primitives::FoundryHeader;
use jsonrpsee::{
    core::RpcResult,
    types::{ErrorObjectOwned, error::INTERNAL_ERROR_CODE},
};
use parking_lot::RwLock;
use reth_ethereum::{
    Block, EthPrimitives, Receipt, TransactionSigned,
    primitives::{
        Account, BlockBody, Bytecode, NodePrimitives, RecoveredBlock, SealedBlock, SealedHeader,
    },
    provider::ProviderError,
    storage::{
        AccountReader, BlockHashReader, BytecodeReader, HashedPostStateProvider,
        StateProofProvider, StateProvider, StateProviderBox, StateRootProvider,
        StorageRootProvider, errors::provider::ProviderResult,
    },
    trie::{
        AccountProof, DecodedMultiProofV2, ExecutionWitnessMode, HashedPostState, HashedStorage,
        MultiProof, MultiProofTargets, MultiProofTargetsV2, StorageMultiProof, StorageProof,
        TrieInput, updates::TrieUpdates,
    },
};
use revm::{
    context::BlockEnv,
    database::{BundleState, DatabaseRef},
    state::AccountInfo,
};
use std::{
    collections::HashMap,
    fmt::{self, Debug, Formatter},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

/// Marks a transaction number as a remote transaction: the number encodes the block number and
/// the index of the transaction in the block, so the local and remote number spaces stay apart.
const REMOTE_TX_FLAG: TxNumber = 1 << 63;
const REMOTE_TX_INDEX_BITS: u32 = 20;

/// Encodes the transaction number of the transaction at `index` in remote block `number`.
pub const fn remote_tx_number(block: u64, index: u64) -> TxNumber {
    REMOTE_TX_FLAG | (block << REMOTE_TX_INDEX_BITS) | index
}

/// Decodes a remote transaction number into its block number and index.
pub const fn decode_remote_tx_number(id: TxNumber) -> Option<(u64, u64)> {
    if id & REMOTE_TX_FLAG == 0 {
        return None;
    }
    let id = id & !REMOTE_TX_FLAG;
    Some((id >> REMOTE_TX_INDEX_BITS, id & ((1 << REMOTE_TX_INDEX_BITS) - 1)))
}

/// Picks how reads block on the remote endpoint: `block_in_place` on a multi-threaded runtime,
/// a plain blocking wait anywhere else.
fn blocking_mode() -> BlockingMode {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            BlockingMode::BlockInPlace
        }
        _ => BlockingMode::Block,
    }
}

/// The RPC network a fork talks to, and how its responses convert into the node's primitives.
pub trait ForkNetwork: Send + Sync + 'static {
    /// The alloy network of the remote endpoint.
    type Network: Network;
    /// The node primitives the responses convert into.
    type Primitives: NodePrimitives;

    /// Returns the hash of a remote block and the number of its uncles.
    fn uncles(response: &<Self::Network as Network>::BlockResponse) -> (B256, usize);

    /// Converts a remote block and its uncles into a sealed block. The transactions of a type the
    /// node cannot execute are left out, as anvil leaves them out of a replay.
    fn block(
        response: <Self::Network as Network>::BlockResponse,
        uncles: Vec<<Self::Network as Network>::BlockResponse>,
    ) -> Result<SealedBlock<<Self::Primitives as NodePrimitives>::Block>, ProviderError>;

    /// Converts a remote receipt, or returns `None` for a transaction type the node cannot
    /// execute.
    fn receipt(
        response: <Self::Network as Network>::ReceiptResponse,
    ) -> Result<Option<<Self::Primitives as NodePrimitives>::Receipt>, ProviderError>;

    /// Converts a remote transaction into the signed transaction and its position in the chain,
    /// when it is mined, or returns `None` for a transaction type the node cannot execute.
    #[expect(clippy::type_complexity)]
    fn transaction(
        response: <Self::Network as Network>::TransactionResponse,
    ) -> Result<
        Option<(<Self::Primitives as NodePrimitives>::SignedTx, Option<TxPosition>)>,
        ProviderError,
    >;

    /// Converts the header of a dumped block into the node's header type.
    fn dump_header(
        header: &FoundryHeader,
    ) -> Result<<Self::Primitives as NodePrimitives>::BlockHeader, ProviderError>;
}

/// Returns the head block header of a state dump in the node's header type. It keeps the dumped
/// hash, also when the node's header type is not the dump's.
pub fn dump_head<F: ForkNetwork>(state: &SerializableState) -> Result<SealedHeader<ForkHeader<F>>> {
    let header =
        &state.head_block().ok_or_else(|| eyre::eyre!("the state dump has no head block"))?.header;
    Ok(SealedHeader::new(F::dump_header(header)?, header.hash_slow()))
}

/// Fetches the uncles of a remote block, which the block response names by hash only.
async fn fetch_uncles<F: ForkNetwork>(
    chain: &RetryProvider<F::Network>,
    block: &<F::Network as Network>::BlockResponse,
) -> Result<Vec<<F::Network as Network>::BlockResponse>> {
    let (hash, count) = F::uncles(block);
    let mut uncles = Vec::with_capacity(count);
    for index in 0..count {
        if let Some(uncle) = chain.get_uncle(BlockId::hash(hash), index as u64).await? {
            uncles.push(uncle);
        }
    }
    Ok(uncles)
}

/// The block hash, block number, and index of a mined transaction.
pub type TxPosition = (B256, u64, u64);

/// How long an `anvil_nodeInfo` probe waits before the endpoint counts as not an anvil node.
/// Anvil waits 500ms; a busy reth-anvil endpoint can take longer to answer, and a fork of it
/// that gives up early runs on the wrong network.
const NODE_INFO_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// The chain ids of zkSync Era and its testnet, whose EraVM bytecode the EVM cannot run.
const ZKSYNC_CHAIN_IDS: [u64; 2] = [324, 300];

/// Rejects a fork of a chain the EVM cannot execute, as anvil does.
pub(crate) fn ensure_fork_network_supported(chain_id: u64) -> Result<()> {
    if ZKSYNC_CHAIN_IDS.contains(&chain_id) {
        eyre::bail!(
            "unsupported fork network (chain id {chain_id}): Anvil's EVM backend cannot execute \
             native EraVM bytecode; use `anvil-zksync` for zkSync Era forks"
        );
    }
    Ok(())
}

/// Asks a fork endpoint whether it is an anvil node, as anvil does: a probe is optional until
/// the endpoint answers once; after that, a failing probe is an error, because it may hide an
/// endpoint reset. A probe that stalls or is rate limited counts as no answer.
pub(crate) struct NodeInfoProbe {
    identified: bool,
    skip: bool,
    timed_out: bool,
}

impl NodeInfoProbe {
    pub(crate) const fn new(identified: bool, skip: bool) -> Self {
        Self { identified, skip, timed_out: false }
    }

    /// Returns whether a probe stalled. Later probes of the endpoint skip it, so a stalling
    /// endpoint delays the startup once.
    pub(crate) const fn timed_out(&self) -> bool {
        self.timed_out
    }

    pub(crate) async fn request<N: Network>(
        &mut self,
        provider: &RetryProvider<N>,
    ) -> Result<Option<NodeInfo>> {
        if self.skip {
            return Ok(None);
        }
        let response = tokio::time::timeout(
            NODE_INFO_PROBE_TIMEOUT,
            provider.raw_request::<_, NodeInfo>("anvil_nodeInfo".into(), ()),
        )
        .await;
        match response {
            Err(_) => {
                self.timed_out = true;
                self.skip = true;
                Ok(None)
            }
            Ok(Ok(info)) => {
                self.identified = true;
                Ok(Some(info))
            }
            Ok(Err(_)) if !self.identified => Ok(None),
            Ok(Err(error)) => {
                Err(error).wrap_err("failed to determine network family from fork endpoint")
            }
        }
    }
}

/// The fork details the RPC namespace reports and changes.
pub trait ForkInfo: Send + Sync + Debug + 'static {
    /// Returns the fork endpoint.
    fn url(&self) -> String;
    /// Returns the chain id of the remote chain.
    fn chain_id(&self) -> u64;
    /// Returns the chain the fork's state comes from.
    fn source_chain_id(&self) -> u64;
    /// Returns the fork block number.
    fn block_number(&self) -> u64;
    /// Returns the fork block hash.
    fn block_hash(&self) -> B256;
    /// Returns the hash of the remote block with the given number.
    fn block_hash_by_number(&self, number: u64) -> ProviderResult<Option<B256>>;
    /// Returns whether an endpoint stands below this fork; a state dump may serve alone.
    fn has_remote(&self) -> bool;
    /// Returns whether the endpoint serves the block with the given number.
    fn remote_serves(&self, number: u64) -> bool;
    /// Returns whether the state at the given block is available for a replay.
    fn has_state_at(&self, number: u64) -> bool;
    /// Returns whether the endpoint identified itself as an anvil node.
    fn is_anvil(&self) -> bool;
    /// Sends a raw JSON-RPC request to the fork endpoint.
    fn forward(&self, method: &str, params: serde_json::Value)
    -> ProviderResult<serde_json::Value>;
    /// Returns the initial backoff of request retries.
    fn retry_backoff(&self) -> Duration;
    /// Replaces the fork endpoint.
    fn set_rpc_url(&self, url: String) -> Result<()>;
}

impl dyn ForkInfo {
    /// Forwards a request to the fork endpoint and decodes its result.
    pub fn forward_json<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> RpcResult<T> {
        let value = self.forward(method, params).map_err(|error| {
            ErrorObjectOwned::owned(
                INTERNAL_ERROR_CODE,
                format!("the fork endpoint failed: {error}"),
                None::<()>,
            )
        })?;
        serde_json::from_value(value).map_err(|error| {
            ErrorObjectOwned::owned(
                INTERNAL_ERROR_CODE,
                format!("the fork endpoint answered: {error}"),
                None::<()>,
            )
        })
    }
}

impl<F: ForkNetwork> ForkInfo for ForkBackend<F> {
    fn url(&self) -> String {
        Self::url(self)
    }

    fn chain_id(&self) -> u64 {
        Self::chain_id(self)
    }

    fn source_chain_id(&self) -> u64 {
        Self::source_chain_id(self)
    }

    fn block_number(&self) -> u64 {
        Self::block_number(self)
    }

    fn block_hash(&self) -> B256 {
        Self::block_hash(self)
    }

    fn block_hash_by_number(&self, number: u64) -> ProviderResult<Option<B256>> {
        Self::block_hash_by_number(self, number)
    }

    fn has_remote(&self) -> bool {
        Self::has_remote(self)
    }

    fn remote_serves(&self, number: u64) -> bool {
        Self::remote_serves(self, number)
    }

    fn has_state_at(&self, number: u64) -> bool {
        Self::has_state_at(self, number)
    }

    fn is_anvil(&self) -> bool {
        self.node_info.is_some()
    }

    fn forward(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> ProviderResult<serde_json::Value> {
        let method = method.to_string();
        self.request(move |chain| async move {
            chain
                .raw_request::<_, serde_json::Value>(method.into(), params)
                .await
                .map_err(Into::into)
        })
    }

    fn retry_backoff(&self) -> Duration {
        self.settings.backoff
    }

    fn set_rpc_url(&self, url: String) -> Result<()> {
        Self::set_rpc_url(self, url)
    }
}

/// Node primitives that can fork a remote chain.
pub trait AnvilPrimitives: NodePrimitives {
    /// The fork network for these primitives.
    type Fork: ForkNetwork<Primitives = Self>;
}

/// The fork backend of a node with the given primitives.
pub type ForkOf<P> = ForkBackend<<P as AnvilPrimitives>::Fork>;

/// The Ethereum fork network.
#[derive(Clone, Copy, Debug, Default)]
pub struct EthereumFork;

impl ForkNetwork for EthereumFork {
    type Network = AnyNetwork;
    type Primitives = EthPrimitives;

    fn uncles(response: &AnyRpcBlock) -> (B256, usize) {
        (response.header.hash, response.uncles.len())
    }

    fn block(
        response: AnyRpcBlock,
        uncles: Vec<AnyRpcBlock>,
    ) -> Result<SealedBlock<Block>, ProviderError> {
        let response = response.into_inner();
        let hash = response.header.hash;
        let header = response.header.inner.into_header_with_defaults();
        let transactions = response
            .transactions
            .into_transactions()
            .filter_map(|tx| tx.into_inner().inner.into_inner().try_into_envelope().ok())
            .map(Into::into)
            .collect();
        let ommers = uncles
            .into_iter()
            .map(|uncle| uncle.into_inner().header.inner.into_header_with_defaults())
            .collect();
        let body =
            alloy_consensus::BlockBody { transactions, ommers, withdrawals: response.withdrawals };
        Ok(SealedBlock::new_unchecked(Block { header, body }, hash))
    }

    fn receipt(response: AnyTransactionReceipt) -> Result<Option<Receipt>, ProviderError> {
        let envelope = &response.inner.inner;
        let Ok(tx_type) = TxType::try_from(envelope.r#type) else { return Ok(None) };
        let receipt = &envelope.inner;
        Ok(Some(Receipt {
            tx_type,
            success: receipt.status(),
            cumulative_gas_used: receipt.cumulative_gas_used(),
            logs: receipt.logs().iter().map(|log| log.inner.clone()).collect(),
        }))
    }

    fn transaction(
        response: AnyRpcTransaction,
    ) -> Result<Option<(TransactionSigned, Option<TxPosition>)>, ProviderError> {
        let response = response.into_inner();
        let position =
            match (response.block_hash, response.block_number, response.transaction_index) {
                (Some(hash), Some(number), Some(index)) => Some((hash, number, index)),
                _ => None,
            };
        let Ok(envelope) = response.inner.into_inner().try_into_envelope() else {
            return Ok(None);
        };
        Ok(Some((envelope.into(), position)))
    }

    fn dump_header(header: &FoundryHeader) -> Result<Header, ProviderError> {
        Ok(header.inner().clone())
    }
}

impl AnvilPrimitives for EthPrimitives {
    type Fork = EthereumFork;
}

/// The header type of a fork network.
pub type ForkHeader<F> = <<F as ForkNetwork>::Primitives as NodePrimitives>::BlockHeader;
/// The block type of a fork network.
type ForkBlock<F> = <<F as ForkNetwork>::Primitives as NodePrimitives>::Block;
/// The receipt type of a fork network.
type ForkReceipt<F> = <<F as ForkNetwork>::Primitives as NodePrimitives>::Receipt;
/// The transaction type of a fork network.
type ForkTx<F> = <<F as ForkNetwork>::Primitives as NodePrimitives>::SignedTx;

/// The remote account state of the accounts the node funds in genesis.
#[derive(Clone, Debug, Default)]
pub struct ForkGenesisAccount {
    /// The remote nonce.
    pub nonce: u64,
    /// The remote code, if any.
    pub code: Option<Bytes>,
}

/// Tells which state keys the local chain has written since the fork.
///
/// Reth records every account and storage change in its history index once a block is persisted,
/// and the in-memory blocks carry their bundle states, so the local chain itself knows which keys
/// it has written. A key the local chain has written at or before the queried block is read from
/// the local state; every other key is read from the remote state at the fork block.
pub trait LocalWrites: Send + Sync {
    /// Returns whether the local chain wrote the account info of `address` at or before `block`.
    fn account_is_local(&self, address: &Address, block: u64) -> ProviderResult<bool>;

    /// Returns whether the local chain wrote `slot` of `address` at or before `block`, or
    /// destroyed the account.
    fn slot_is_local(
        &self,
        address: &Address,
        slot: &StorageKey,
        block: u64,
    ) -> ProviderResult<bool>;
}

/// Remote state the node has read since the last block, to be copied into the local database.
///
/// The engine validates blocks against the local database directly, so every remote account,
/// slot, and bytecode the block builder read must be in the local database before the engine
/// executes the block. The copies carry the fork block as their history entry, so later reads
/// treat them as local state.
#[derive(Debug, Default)]
pub struct RemoteReads {
    /// Remote accounts by address.
    pub accounts: HashMap<Address, Account>,
    /// Remote bytecodes by hash.
    pub codes: HashMap<B256, Bytecode>,
    /// Remote storage slots with a non-zero value.
    pub storage: HashMap<(Address, StorageKey), StorageValue>,
}

impl RemoteReads {
    /// Returns whether nothing was read.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.codes.is_empty() && self.storage.is_empty()
    }
}

/// Connection settings of the fork endpoint.
#[derive(Clone, Debug)]
pub struct ForkSettings {
    /// The fork endpoints. The first one serves the requests.
    pub urls: Vec<String>,
    /// Extra HTTP headers.
    pub headers: Vec<String>,
    /// Request timeout.
    pub timeout: Duration,
    /// Number of retries for failed requests.
    pub retries: u32,
    /// Initial backoff for retries.
    pub backoff: Duration,
    /// Assumed compute units per second of the endpoint.
    pub compute_units_per_second: u64,
    /// Skip the on-disk cache.
    pub no_storage_caching: bool,
    /// Fetch state by block number instead of block hash.
    pub state_by_number: bool,
    /// The instance id of the endpoint, when it is an anvil node: a reset of that node is another
    /// chain behind the same URL, so its cached state must not be reused.
    pub instance_id: Option<B256>,
    /// How accounts are read from the endpoint. A Tempo endpoint reports a placeholder for native
    /// balances, so accounts must come from `eth_getAccountInfo`.
    pub account_fetch_policy: AccountFetchPolicy,
}

impl ForkSettings {
    fn provider<N: alloy_network::Network>(&self, url: &str) -> Result<RetryProvider<N>> {
        ProviderBuilder::<N>::new(url)
            .timeout(self.timeout)
            .initial_backoff(self.backoff.as_millis() as u64)
            .compute_units_per_second(self.compute_units_per_second)
            .max_retry(self.retries)
            .headers(self.headers.clone())
            .build()
            .wrap_err("failed to establish provider to fork url")
    }

    fn cache_path(&self, chain_id: u64, block: u64, url: &str) -> Option<PathBuf> {
        if self.no_storage_caching {
            return None;
        }
        let rpc_url_hash = alloy_primitives::hex::encode(keccak256(url));
        Some(
            Config::foundry_block_cache_file(chain_id, block)?
                .with_file_name(format!("storage-{rpc_url_hash}.json")),
        )
    }

    fn source_id(&self) -> B256 {
        let mut encoded = Vec::new();
        for parts in [&self.urls, &self.headers] {
            encoded.extend_from_slice(&(parts.len() as u64).to_be_bytes());
            for part in parts {
                encoded.extend_from_slice(&(part.len() as u64).to_be_bytes());
                encoded.extend_from_slice(part.as_bytes());
            }
        }
        if let Some(instance_id) = self.instance_id {
            encoded.extend_from_slice(instance_id.as_slice());
        }
        keccak256(encoded)
    }
}

/// The transactions a fork at a transaction hash replays into the first local block: the ones
/// before the target in its block, and the target itself.
pub struct ForkReplay<F: ForkNetwork = EthereumFork> {
    /// The header of the block the transactions came from.
    pub header: SealedHeader<ForkHeader<F>>,
    /// The transactions, in block order.
    pub transactions: Vec<<F::Primitives as NodePrimitives>::SignedTx>,
}

/// The remote side of a fork.
pub struct ForkBackend<F: ForkNetwork = EthereumFork> {
    settings: ForkSettings,
    /// The transactions to replay at startup, for a fork at a transaction hash.
    replay: RwLock<Option<ForkReplay<F>>>,
    url: RwLock<String>,
    chain_id: u64,
    /// The chain the fork's state comes from: the endpoint's chain, or the chain an anvil
    /// endpoint forks in turn.
    source_chain_id: u64,
    header: SealedHeader<ForkHeader<F>>,
    gas_price: u128,
    /// The remote state at the fork block.
    state: RwLock<SharedBackend>,
    /// The remote state at blocks below the fork block, by block number.
    history: RwLock<HashMap<u64, SharedBackend>>,
    /// The chain reader for blocks below the fork block.
    chain: RwLock<RetryProvider<F::Network>>,
    /// Remote bytecodes by hash.
    codes: RwLock<HashMap<B256, Bytecode>>,
    reads: RwLock<RemoteReads>,
    blocks: RwLock<HashMap<B256, Arc<SealedBlock<ForkBlock<F>>>>>,
    hashes: RwLock<HashMap<u64, B256>>,
    receipts: RwLock<HashMap<B256, Arc<Vec<ForkReceipt<F>>>>>,
    /// The state dump this fork serves instead of an endpoint, if any.
    dump: Option<DumpHistory>,
    /// What the endpoint reported about itself, when it is an anvil node.
    node_info: Option<NodeInfo>,
}

/// What a state dump provides as the chain below its head block.
struct DumpHistory {
    /// The blocks and transactions of the dump, for the next dump.
    blocks: Vec<SerializableBlock>,
    transactions: Vec<SerializableTransaction>,
    /// The state at every block, by block hash.
    snapshots: HashMap<B256, Arc<StateSnapshot>>,
    /// The senders of the transactions of every block, by block hash.
    senders: HashMap<B256, Vec<Address>>,
    /// The block hash and index of every mined transaction.
    positions: HashMap<B256, (B256, u64)>,
    /// The transactions whose signature does not recover their sender.
    impersonated: Vec<(B256, Address)>,
    /// The fork block of the endpoint below the dump, when the dump sits on top of a fork.
    remote_head: Option<u64>,
}

/// The state a fork serves under the local state: a remote endpoint's, the one a state dump
/// recorded at the block, or none.
#[derive(Clone)]
pub enum RemoteState {
    /// The state of a remote endpoint.
    Shared(SharedBackend),
    /// The state a dump recorded at the block.
    Snapshot(Arc<StateSnapshot>),
    /// No state below the local one.
    None,
}

impl RemoteState {
    fn with_blocking_mode(self, mode: BlockingMode) -> Self {
        match self {
            Self::Shared(backend) => Self::Shared(backend.with_blocking_mode(mode)),
            other => other,
        }
    }

    fn basic(&self, address: Address) -> ProviderResult<Option<AccountInfo>> {
        match self {
            Self::Shared(backend) => backend.basic_ref(address).map_err(ProviderError::other),
            Self::Snapshot(snapshot) => {
                snapshot.accounts.get(&address).map(snapshot_account_info).transpose()
            }
            Self::None => Ok(None),
        }
    }

    fn storage(&self, address: Address, slot: U256) -> ProviderResult<StorageValue> {
        match self {
            Self::Shared(backend) => {
                backend.storage_ref(address, slot).map_err(ProviderError::other)
            }
            Self::Snapshot(snapshot) => Ok(snapshot
                .storage
                .get(&address)
                .and_then(|storage| storage.get(&slot))
                .copied()
                .unwrap_or_default()),
            Self::None => Ok(StorageValue::ZERO),
        }
    }
}

/// Converts a snapshot account into revm's account info.
fn snapshot_account_info(account: &SnapshotAccount) -> ProviderResult<AccountInfo> {
    let code = match &account.code {
        Some(code) => Some(json_convert(code)?),
        None => None,
    };
    Ok(AccountInfo {
        balance: account.balance,
        nonce: account.nonce,
        code_hash: account.code_hash,
        code,
        ..Default::default()
    })
}

impl<F: ForkNetwork> Debug for ForkBackend<F> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForkBackend")
            .field("url", &*self.url.read())
            .field("chain_id", &self.chain_id)
            .field("block_number", &self.header.number())
            .field("block_hash", &self.header.hash())
            .finish_non_exhaustive()
    }
}

impl<F: ForkNetwork> ForkBackend<F> {
    /// Connects to the fork endpoint, resolves the fork block, and fetches the remote state of
    /// the genesis accounts.
    pub async fn setup(
        config: &NodeConfig,
    ) -> Result<(Arc<Self>, Vec<(Address, ForkGenesisAccount)>)> {
        let mut settings = config.fork_settings();
        let url = settings.urls.first().cloned().ok_or_else(|| eyre::eyre!("no fork url"))?;
        // The node runtime drives this provider. The backends get their own providers below, so
        // their connections live on the backend thread: a read may block the node runtime while
        // it waits for the backend, and a connection driven by the blocked runtime would stall.
        let provider = settings.provider::<alloy_network::AnyNetwork>(&url)?;
        let chain = settings.provider::<F::Network>(&url)?;

        let mut probe = NodeInfoProbe::new(
            config.is_anvil_endpoint(&url),
            config.no_fork_node_info || config.is_stalled_endpoint(&url),
        );
        let node_info_before = probe.request(&provider).await?;
        let chain_id = match config.fork_chain_id {
            Some(chain_id) => chain_id,
            None => provider.get_chain_id().await.wrap_err("failed to fetch network chain ID")?,
        };
        ensure_fork_network_supported(chain_id)?;
        if settings.urls.len() > 1 {
            eyre::ensure!(
                config.fork_chain_id.is_none(),
                "multiple fork URLs cannot be validated with --fork-chain-id; remove \
                 --fork-chain-id to validate every endpoint"
            );
            for other in &settings.urls[1..] {
                let other_provider = settings.provider::<alloy_network::AnyNetwork>(other)?;
                // Mirrors are probed too, so their identity stays strict across resets.
                let mut other_probe =
                    NodeInfoProbe::new(config.is_anvil_endpoint(other), config.no_fork_node_info);
                if other_probe.request(&other_provider).await?.is_some() {
                    config.mark_anvil_endpoint(other);
                }
                let other_chain_id = other_provider
                    .get_chain_id()
                    .await
                    .wrap_err_with(|| format!("failed to fetch the chain ID of {other}"))?;
                ensure_fork_network_supported(other_chain_id)?;
                eyre::ensure!(
                    other_chain_id == chain_id,
                    "fork endpoints must use the same chain ID: expected {chain_id}, got \
                     {other_chain_id} from {other}"
                );
            }
        }

        let mut replay_target = None;
        let block_number = match config.fork_choice.or_else(|| {
            config
                .fork_urls
                .first()
                .and_then(|fork| fork.block)
                .map(|block| ForkChoice::Block(block as i128))
        }) {
            Some(ForkChoice::Block(number)) if number < 0 => {
                let latest = provider.get_block_number().await?;
                number.saturating_add(latest as i128).max(0) as u64
            }
            Some(ForkChoice::Block(number)) => number as u64,
            Some(ForkChoice::Transaction(hash)) => {
                let tx = chain
                    .get_transaction_by_hash(hash)
                    .await?
                    .ok_or_else(|| eyre::eyre!("transaction {hash} not found on the fork"))?;
                let Some((_, position)) = F::transaction(tx)? else {
                    eyre::bail!("transaction {hash} has a type the node cannot execute");
                };
                let Some((_, number, _)) = position else {
                    eyre::bail!("transaction {hash} is not mined yet");
                };
                if number == 0 {
                    eyre::bail!("transaction {hash} is in the genesis block");
                }
                replay_target = Some((number, hash));
                number - 1
            }
            None => find_latest_fork_block(&provider)
                .await
                .wrap_err("failed to get fork block number")?,
        };

        let block = chain.get_block_by_number(block_number.into()).full().await?;
        let Some(block) = block else {
            let mut message = format!("Failed to get block for block number: {block_number}");
            if let Ok(latest) = provider.get_block_number().await {
                message.push_str(&format!("\nlatest block number: {latest}"));
                if block_number <= latest {
                    message.push('\n');
                    message.push_str(foundry_common::NON_ARCHIVE_NODE_WARNING);
                }
            }
            eyre::bail!("{message}");
        };
        let block = F::block(block, Vec::new())?;
        let hash = block.hash();
        let header = block.sealed_header().clone();
        let gas_price =
            provider.get_gas_price().await.unwrap_or(crate::config::INITIAL_BASE_FEE as u128);
        // An endpoint that identified itself must still answer: a failure now hides a reset.
        let node_info = probe.request(&provider).await?.or(node_info_before);
        let mut source_chain_id = chain_id;
        let network_profile = node_info
            .as_ref()
            .and_then(|info| {
                NetworkConfigs::from_rpc_identity_profile_with_fallback(
                    chain_id,
                    Some(info.network.as_deref()),
                    None,
                )
                .ok()
                .flatten()
            })
            .unwrap_or_default();
        if node_info.is_some() {
            config.mark_anvil_endpoint(&url);
            let metadata = tokio::time::timeout(
                NODE_INFO_PROBE_TIMEOUT,
                provider.raw_request::<_, Metadata>("anvil_metadata".into(), ()),
            )
            .await
            .ok()
            .and_then(Result::ok);
            if let Some(metadata) = metadata {
                settings.instance_id = Some(metadata.instance_id);
                if config.fork_chain_id.is_none()
                    && let Some(forked) = metadata.forked_network
                {
                    source_chain_id = forked.chain_id;
                }
            }
        }
        settings.account_fetch_policy =
            account_fetch_policy_for_source(source_chain_id, network_profile);
        let replay = match replay_target {
            Some((number, target)) => {
                let block = chain
                    .get_block_by_number(number.into())
                    .full()
                    .await?
                    .ok_or_else(|| eyre::eyre!("failed to get block {number} from the fork"))?;
                // The block holds only the transactions the node can execute, so the prefix ends
                // at the target's position among those.
                let block = F::block(block, Vec::new())?;
                let mut transactions = Vec::new();
                for tx in block.body().transactions() {
                    transactions.push(tx.clone());
                    if *tx.tx_hash() == target {
                        break;
                    }
                }
                Some(ForkReplay { header: block.sealed_header().clone(), transactions })
            }
            None => None,
        };

        // The genesis accounts keep their remote nonce and code and get the configured balance.
        let mut genesis_accounts = Vec::new();
        let addresses: Vec<Address> = config
            .genesis_accounts
            .iter()
            .map(|wallet| wallet.address())
            .chain(config.funded_accounts.keys().copied())
            .collect();
        for address in addresses {
            let (nonce, code) = tokio::try_join!(
                provider.get_transaction_count(address).block_id(hash.into()),
                provider.get_code_at(address).block_id(hash.into()),
            )
            .wrap_err_with(|| format!("failed to fetch account {address} from the fork"))?;
            let code = (!code.is_empty()).then_some(code);
            genesis_accounts.push((address, ForkGenesisAccount { nonce, code }));
        }

        let meta =
            BlockchainDbMeta::new(block_env_from_header::<BlockEnv>(header.header()), url.clone())
                .with_fork_identity(hash, settings.source_id())
                .with_account_fetch_policy(settings.account_fetch_policy);
        let db = BlockchainDb::new(meta, settings.cache_path(chain_id, block_number, &url));
        drop((provider, chain));
        let state = spawn_state_backend(
            Arc::new(settings.provider::<alloy_network::AnyNetwork>(&url)?),
            db,
            ForkAnchor::new(block_number, hash),
            settings.state_by_number,
        )?;
        let chain = settings.provider::<F::Network>(&url)?;

        Ok((
            Arc::new(Self {
                settings,
                replay: RwLock::new(replay),
                url: RwLock::new(url),
                chain_id,
                source_chain_id,
                header,
                gas_price,
                state: RwLock::new(state),
                history: RwLock::new(HashMap::new()),
                chain: RwLock::new(chain),
                codes: RwLock::new(HashMap::new()),
                reads: RwLock::new(RemoteReads::default()),
                blocks: RwLock::new(HashMap::new()),
                hashes: RwLock::new(HashMap::from([(block_number, hash)])),
                receipts: RwLock::new(HashMap::new()),
                dump: None,
                node_info,
            }),
            genesis_accounts,
        ))
    }

    /// Serves the blocks, transactions, receipts, and states of a state dump as the chain below
    /// its head block, which becomes the local genesis. A dump has no endpoint: what it does not
    /// hold does not exist.
    pub fn from_dump(
        config: &NodeConfig,
        state: &SerializableState,
        head: &SealedHeader<ForkHeader<F>>,
    ) -> Result<Arc<Self>> {
        let mut settings = config.fork_settings();
        settings.retries = 0;
        settings.timeout = Duration::from_secs(1);
        settings.no_storage_caching = true;
        let url = "http://dump.invalid".to_string();
        let chain_id = config.get_chain_id();
        let gas_price =
            head.base_fee_per_gas().map_or(crate::config::INITIAL_BASE_FEE as u128, u128::from);

        let mut blocks = HashMap::new();
        let mut hashes = HashMap::new();
        let mut receipts = HashMap::new();
        let mut codes = HashMap::new();
        let dump = Self::dump_history(
            state,
            head,
            None,
            &mut blocks,
            &mut hashes,
            &mut receipts,
            &mut codes,
        )?;

        let meta =
            BlockchainDbMeta::new(block_env_from_header::<BlockEnv>(head.header()), url.clone())
                .with_account_fetch_policy(settings.account_fetch_policy);
        let (shared, handler) = SharedBackend::new(
            Arc::new(settings.provider::<alloy_network::AnyNetwork>(&url)?),
            BlockchainDb::new(meta, None),
            None,
        );
        spawn_backend_handler(handler)?;
        let chain = settings.provider::<F::Network>(&url)?;
        Ok(Arc::new(Self {
            settings,
            replay: RwLock::new(None),
            url: RwLock::new(String::new()),
            chain_id,
            source_chain_id: chain_id,
            header: head.clone(),
            gas_price,
            state: RwLock::new(shared),
            history: RwLock::new(HashMap::new()),
            chain: RwLock::new(chain),
            codes: RwLock::new(codes),
            reads: RwLock::new(RemoteReads::default()),
            blocks: RwLock::new(blocks),
            hashes: RwLock::new(hashes),
            receipts: RwLock::new(receipts),
            dump: Some(dump),
            node_info: None,
        }))
    }

    /// Reads a dump into the caches: its blocks, receipts, transaction positions, senders,
    /// codes, and states.
    fn dump_history(
        state: &SerializableState,
        head: &SealedHeader<ForkHeader<F>>,
        remote_head: Option<u64>,
        blocks: &mut HashMap<B256, Arc<SealedBlock<ForkBlock<F>>>>,
        hashes: &mut HashMap<u64, B256>,
        receipts: &mut HashMap<B256, Arc<Vec<ForkReceipt<F>>>>,
        codes: &mut HashMap<B256, Bytecode>,
    ) -> Result<DumpHistory> {
        let mut senders = HashMap::new();
        let mut impersonated = Vec::new();
        // One entry per block, as anvil stores them by hash.
        let mut dumped_blocks = HashMap::new();
        for block in &state.blocks {
            let header = F::dump_header(&block.header)?;
            let transactions = block
                .transactions
                .iter()
                .map(|transaction| transaction.transaction())
                .collect::<Vec<_>>();
            let body = json_convert(&serde_json::json!({
                "transactions": transactions,
                "ommers": block.ommers,
                "withdrawals": block.withdrawals,
            }))?;
            // A dumped block keeps its hash, also when the node's header type is not the dump's.
            let sealed = SealedBlock::new_unchecked(
                <ForkBlock<F> as reth_ethereum::primitives::Block>::new(header, body),
                block.header.hash_slow(),
            );
            let hash = sealed.hash();
            let mut block_senders = Vec::with_capacity(block.transactions.len());
            for (tx, dumped) in sealed.body().transactions().iter().zip(&block.transactions) {
                let recovered = tx.recover_signer().ok();
                let sender = match (dumped.impersonated_sender(), recovered) {
                    (Some(sender), _) => {
                        if recovered != Some(sender) {
                            impersonated.push((*tx.tx_hash(), sender));
                        }
                        sender
                    }
                    (None, Some(sender)) => sender,
                    (None, None) => eyre::bail!(
                        "transaction {} of block {} in the state dump has no sender",
                        tx.tx_hash(),
                        sealed.number()
                    ),
                };
                block_senders.push(sender);
            }
            // Blocks above the head stay reachable by hash, as anvil keeps them, but no number
            // names them.
            if sealed.number() <= head.number() {
                hashes.insert(sealed.number(), hash);
            }
            senders.insert(hash, block_senders);
            blocks.insert(hash, Arc::new(sealed));
            dumped_blocks.entry(hash).or_insert_with(|| block.clone());
        }
        hashes.insert(head.number(), head.hash());
        // Anvil's order: by number, the canonical block last among its height, then by hash.
        let mut dumped_blocks = dumped_blocks.into_iter().collect::<Vec<_>>();
        dumped_blocks.sort_unstable_by_key(|(hash, block)| {
            let number = block.header.number;
            (number, hashes.get(&number) == Some(hash), *hash)
        });
        let dumped_blocks = dumped_blocks.into_iter().map(|(_, block)| block).collect();
        let mut dumped_transactions = state
            .transactions
            .iter()
            .map(|transaction| {
                (
                    (
                        transaction.block_number,
                        transaction.info.transaction_index,
                        transaction.info.transaction_hash,
                    ),
                    transaction.clone(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
            .into_values()
            .collect::<Vec<_>>();
        dumped_transactions.dedup_by_key(|transaction| transaction.info.transaction_hash);

        let mut by_block = HashMap::<B256, Vec<(u64, ForkReceipt<F>)>>::new();
        let mut positions = HashMap::new();
        for transaction in &state.transactions {
            let receipt: ForkReceipt<F> = json_convert(&transaction.receipt)?;
            by_block
                .entry(transaction.block_hash)
                .or_default()
                .push((transaction.info.transaction_index, receipt));
            positions.insert(
                transaction.info.transaction_hash,
                (transaction.block_hash, transaction.info.transaction_index),
            );
        }
        for hash in blocks.keys() {
            let mut block_receipts = by_block.remove(hash).unwrap_or_default();
            block_receipts.sort_by_key(|(index, _)| *index);
            receipts.insert(
                *hash,
                Arc::new(block_receipts.into_iter().map(|(_, receipt)| receipt).collect()),
            );
        }

        for record in state.accounts.values() {
            if !record.code.is_empty() {
                codes.insert(keccak256(&record.code), Bytecode::new_raw(record.code.clone()));
            }
        }
        let mut snapshots = HashMap::new();
        if let Some(states) = &state.historical_states {
            for (hash, snapshot) in &states.0 {
                for account in snapshot.accounts.values() {
                    if let Some(code) = &account.code {
                        codes.insert(account.code_hash, Bytecode(json_convert(code)?));
                    }
                }
                snapshots.insert(*hash, Arc::new(snapshot.clone()));
            }
        }
        Ok(DumpHistory {
            blocks: dumped_blocks,
            transactions: dumped_transactions,
            snapshots,
            senders,
            positions,
            impersonated,
            remote_head,
        })
    }

    /// Puts a state dump on top of this fork: the dump's head block becomes the local genesis,
    /// the dump's blocks above the fork block come from the dump, and the chain below the fork
    /// block stays with the endpoint.
    pub fn into_dump_fork(
        self,
        state: &SerializableState,
        head: &SealedHeader<ForkHeader<F>>,
    ) -> Result<Self> {
        let remote_head = self.header.number();
        let mut blocks = self.blocks.into_inner();
        let mut hashes = self.hashes.into_inner();
        let mut receipts = self.receipts.into_inner();
        let mut codes = self.codes.into_inner();
        let dump = Self::dump_history(
            state,
            head,
            Some(remote_head),
            &mut blocks,
            &mut hashes,
            &mut receipts,
            &mut codes,
        )?;
        Ok(Self {
            settings: self.settings,
            replay: RwLock::new(None),
            url: self.url,
            chain_id: self.chain_id,
            source_chain_id: self.source_chain_id,
            header: head.clone(),
            gas_price: self.gas_price,
            state: self.state,
            history: self.history,
            chain: self.chain,
            codes: RwLock::new(codes),
            reads: self.reads,
            blocks: RwLock::new(blocks),
            hashes: RwLock::new(hashes),
            receipts: RwLock::new(receipts),
            dump: Some(dump),
            node_info: None,
        })
    }

    /// Returns whether this fork serves a state dump, on top of an endpoint or alone.
    pub const fn is_dump(&self) -> bool {
        self.dump.is_some()
    }

    /// Returns whether an endpoint stands below this fork.
    pub fn has_remote(&self) -> bool {
        self.dump.as_ref().is_none_or(|dump| dump.remote_head.is_some())
    }

    /// Returns whether the endpoint serves the block with the given number.
    pub fn remote_serves(&self, number: u64) -> bool {
        match &self.dump {
            None => true,
            Some(dump) => dump.remote_head.is_some_and(|head| number <= head),
        }
    }

    /// Returns whether the state at the given block is available for a replay: the local chain
    /// holds the fork block and the blocks above it, the endpoint serves the blocks it stands
    /// below, and a dump the blocks it carries a state snapshot for.
    pub fn has_state_at(&self, number: u64) -> bool {
        if number >= self.header.number() || self.remote_serves(number) {
            return true;
        }
        let Some(dump) = &self.dump else { return true };
        self.hashes.read().get(&number).is_some_and(|hash| dump.snapshots.contains_key(hash))
    }

    /// Returns the blocks and transactions of the state dump this fork serves, if any.
    pub fn dumped_history(&self) -> (Vec<SerializableBlock>, Vec<SerializableTransaction>) {
        match &self.dump {
            Some(dump) => (dump.blocks.clone(), dump.transactions.clone()),
            None => (Vec::new(), Vec::new()),
        }
    }

    /// Returns the states of the dump this fork serves, oldest block first.
    pub fn dumped_snapshots(&self) -> Vec<(B256, StateSnapshot)> {
        let Some(dump) = &self.dump else { return Vec::new() };
        let hashes = self.hashes.read();
        let mut numbers = hashes.iter().map(|(number, hash)| (*number, *hash)).collect::<Vec<_>>();
        numbers.sort_unstable();
        numbers
            .into_iter()
            .filter_map(|(_, hash)| {
                dump.snapshots.get(&hash).map(|state| (hash, (**state).clone()))
            })
            .collect()
    }

    /// Returns the transactions of the dump whose signature does not recover their sender, with
    /// the sender the dump names.
    pub fn impersonated_transactions(&self) -> Vec<(B256, Address)> {
        self.dump.as_ref().map(|dump| dump.impersonated.clone()).unwrap_or_default()
    }

    /// Returns the fork endpoint.
    pub fn url(&self) -> String {
        self.url.read().clone()
    }

    /// Takes the transactions to replay at startup, if the fork is at a transaction hash.
    pub fn take_replay(&self) -> Option<ForkReplay<F>> {
        self.replay.write().take()
    }

    /// Returns the timestamp of the block the fork replays, if the fork is at a transaction hash.
    pub fn replay_timestamp(&self) -> Option<u64> {
        self.replay.read().as_ref().map(|replay| replay.header.timestamp())
    }

    /// Returns the chain id of the remote chain.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the chain the fork's state comes from, which differs from the endpoint's chain id
    /// when the endpoint is an anvil fork with another chain id.
    pub const fn source_chain_id(&self) -> u64 {
        self.source_chain_id
    }

    /// Returns the fork block number.
    pub fn block_number(&self) -> u64 {
        self.header.number()
    }

    /// Returns the fork block hash.
    pub fn block_hash(&self) -> B256 {
        self.header.hash()
    }

    /// Returns the fork block header.
    pub const fn header(&self) -> &SealedHeader<ForkHeader<F>> {
        &self.header
    }

    /// Returns the gas price reported by the remote endpoint at startup.
    pub const fn gas_price(&self) -> u128 {
        self.gas_price
    }

    /// Returns the connection settings.
    pub const fn settings(&self) -> &ForkSettings {
        &self.settings
    }

    /// Returns whether the block with the given number is below the fork block.
    pub fn predates_fork(&self, number: u64) -> bool {
        number < self.header.number()
    }

    /// Replaces the fork endpoint. The state fetched so far stays cached.
    pub fn set_rpc_url(&self, url: String) -> Result<()> {
        let provider = self.settings.provider::<alloy_network::AnyNetwork>(&url)?;
        let chain = self.settings.provider::<F::Network>(&url)?;
        let db = {
            let state = self.state.read();
            let meta = BlockchainDbMeta::new(
                block_env_from_header::<BlockEnv>(self.header.header()),
                url.clone(),
            )
            .with_fork_identity(self.header.hash(), self.settings.source_id())
            .with_account_fetch_policy(self.settings.account_fetch_policy);
            let db = BlockchainDb::new(
                meta,
                self.settings.cache_path(self.chain_id, self.header.number(), &url),
            );
            // Carry over the fetched data.
            *db.accounts().write() = state.accounts();
            *db.storage().write() = state.storage();
            *db.block_hashes().write() = state.block_hashes();
            db
        };
        let state = spawn_state_backend(
            Arc::new(provider),
            db,
            ForkAnchor::new(self.header.number(), self.header.hash()),
            self.settings.state_by_number,
        )?;
        *self.state.write() = state;
        *self.chain.write() = chain;
        self.history.write().clear();
        *self.url.write() = url;
        Ok(())
    }

    /// Returns the remote state at the given block, which must not be above the fork block.
    pub fn state_at(&self, number: u64) -> ProviderResult<RemoteState> {
        if let Some(dump) = &self.dump
            && !self.remote_serves(number)
        {
            // At the dump's head and above, the endpoint's state at its fork block stands under
            // the local state; without an endpoint, nothing does.
            if number >= self.header.number() {
                return Ok(match dump.remote_head {
                    Some(_) => RemoteState::Shared(self.state.read().clone()),
                    None => RemoteState::None,
                });
            }
            let hash = self.hashes.read().get(&number).copied();
            return Ok(match hash.and_then(|hash| dump.snapshots.get(&hash)) {
                Some(snapshot) => RemoteState::Snapshot(snapshot.clone()),
                None => RemoteState::None,
            });
        }
        if number >= self.header.number() {
            return Ok(RemoteState::Shared(self.state.read().clone()));
        }
        if let Some(backend) = self.history.read().get(&number) {
            return Ok(RemoteState::Shared(backend.clone()));
        }
        let header = self
            .header_by_number(number)?
            .ok_or(ProviderError::HeaderNotFound(BlockHashOrNumber::Number(number)))?;
        let url = self.url();
        let meta = BlockchainDbMeta::new(block_env_from_header::<BlockEnv>(&header), url.clone())
            .with_account_fetch_policy(self.settings.account_fetch_policy);
        let db = BlockchainDb::new(meta, self.settings.cache_path(self.chain_id, number, &url));
        let provider = self
            .settings
            .provider::<alloy_network::AnyNetwork>(&url)
            .map_err(|error| ProviderError::other(std::io::Error::other(format!("{error:#}"))))?;
        let backend = SharedBackend::spawn_backend_thread(
            Arc::new(provider),
            db,
            Some(BlockId::number(number)),
        );
        self.history.write().insert(number, backend.clone());
        Ok(RemoteState::Shared(backend))
    }

    /// Returns the remote bytecode with the given hash: one a remote account read fetched, or
    /// one the endpoint serves through `debug_codeByHash`, as anvil endpoints do.
    pub fn code_by_hash(&self, hash: &B256) -> ProviderResult<Option<Bytecode>> {
        if let Some(code) = self.codes.read().get(hash) {
            return Ok(Some(code.clone()));
        }
        if !self.has_remote() {
            return Ok(None);
        }
        let hash = *hash;
        let code = self.request(move |chain| async move {
            chain
                .raw_request::<_, Option<Bytes>>("debug_codeByHash".into(), (hash, None::<BlockId>))
                .await
                .map_err(Into::into)
        });
        // Endpoints without the method serve no code by hash.
        let Ok(Some(code)) = code else { return Ok(None) };
        let code = Bytecode::new_raw(code);
        self.codes.write().insert(hash, code.clone());
        Ok(Some(code))
    }

    /// Takes the remote reads made since the last call.
    pub fn take_reads(&self) -> RemoteReads {
        std::mem::take(&mut *self.reads.write())
    }

    fn record_code(&self, info: &AccountInfo) -> Option<Bytecode> {
        let code = info.code.as_ref().filter(|code| !code.is_empty())?;
        let code = Bytecode(code.clone());
        self.codes.write().entry(info.code_hash).or_insert_with(|| code.clone());
        Some(code)
    }

    /// Records a remote account read at the fork block.
    fn record_account(&self, address: Address, info: &AccountInfo, account: &Account) {
        let mut reads = self.reads.write();
        reads.accounts.entry(address).or_insert(*account);
        if let Some(code) = self.record_code(info) {
            reads.codes.entry(info.code_hash).or_insert(code);
        }
    }

    /// Records a remote storage read at the fork block.
    fn record_slot(&self, address: Address, slot: StorageKey, value: StorageValue) {
        if !value.is_zero() {
            self.reads.write().storage.entry((address, slot)).or_insert(value);
        }
    }

    /// Runs a request against the remote endpoint and blocks on the result.
    fn request<T, Fut>(
        &self,
        request: impl FnOnce(RetryProvider<F::Network>) -> Fut,
    ) -> ProviderResult<T>
    where
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Debug + Send + 'static,
    {
        if !self.has_remote() {
            return Err(ProviderError::other(std::io::Error::other(
                "a chain loaded from a state dump has no fork endpoint",
            )));
        }
        let future = request(self.chain.read().clone());
        let mut state = self.state.read().with_blocking_mode(blocking_mode());
        state.do_any_request(future).map_err(ProviderError::other)
    }

    fn cache_block(
        &self,
        response: (
            <F::Network as Network>::BlockResponse,
            Vec<<F::Network as Network>::BlockResponse>,
        ),
    ) -> ProviderResult<Arc<SealedBlock<ForkBlock<F>>>> {
        let (response, uncles) = response;
        let block = Arc::new(F::block(response, uncles)?);
        let hash = block.hash();
        self.hashes.write().insert(block.number(), hash);
        self.blocks.write().insert(hash, block.clone());
        Ok(block)
    }

    /// Returns what the endpoint reported about itself at setup, when it is an anvil node, so a
    /// fork adopts its hardfork, as anvil does.
    pub const fn node_info(&self) -> Option<&NodeInfo> {
        self.node_info.as_ref()
    }

    /// Removes the cache file of the remote state at the fork block, if any.
    pub fn remove_cache(&self) {
        let path = self.settings.cache_path(self.chain_id, self.header.number(), &self.url.read());
        if let Some(path) = path {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Returns the remote account proof at the given block.
    pub fn account_proof(
        &self,
        address: Address,
        slots: &[B256],
        block: u64,
    ) -> ProviderResult<AccountProof> {
        let keys = slots.to_vec();
        let response = self.request(move |chain| async move {
            chain.get_proof(address, keys).block_id(block.into()).await.map_err(Into::into)
        })?;
        Ok(AccountProof::from_eip1186_proof(response))
    }

    /// Returns the remote block with the given hash.
    pub fn block_by_hash(
        &self,
        hash: B256,
    ) -> ProviderResult<Option<Arc<SealedBlock<ForkBlock<F>>>>> {
        if let Some(block) = self.blocks.read().get(&hash) {
            return Ok(Some(block.clone()));
        }
        if !self.has_remote() {
            return Ok(None);
        }
        let block = self.request(move |chain| async move {
            let Some(block) = chain.get_block_by_hash(hash).full().await? else { return Ok(None) };
            let uncles = fetch_uncles::<F>(&chain, &block).await?;
            Ok(Some((block, uncles)))
        })?;
        block.map(|block| self.cache_block(block)).transpose()
    }

    /// Returns the remote block with the given number.
    pub fn block_by_number(
        &self,
        number: u64,
    ) -> ProviderResult<Option<Arc<SealedBlock<ForkBlock<F>>>>> {
        if let Some(hash) = self.hashes.read().get(&number)
            && let Some(block) = self.blocks.read().get(hash)
        {
            return Ok(Some(block.clone()));
        }
        if !self.remote_serves(number) {
            return Ok(None);
        }
        let block = self.request(move |chain| async move {
            let Some(block) = chain.get_block_by_number(number.into()).full().await? else {
                return Ok(None);
            };
            let uncles = fetch_uncles::<F>(&chain, &block).await?;
            Ok(Some((block, uncles)))
        })?;
        block.map(|block| self.cache_block(block)).transpose()
    }

    /// Returns the remote block for the given id.
    pub fn block(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Arc<SealedBlock<ForkBlock<F>>>>> {
        match id {
            BlockHashOrNumber::Hash(hash) => self.block_by_hash(hash),
            BlockHashOrNumber::Number(number) => self.block_by_number(number),
        }
    }

    /// Returns the remote block for the given id with recovered senders.
    pub fn recovered_block(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<RecoveredBlock<ForkBlock<F>>>> {
        let Some(block) = self.block(id)? else { return Ok(None) };
        if let Some(senders) = self.dump.as_ref().and_then(|dump| dump.senders.get(&block.hash())) {
            return Ok(Some(RecoveredBlock::new_sealed((*block).clone(), senders.clone())));
        }
        let block = (*block)
            .clone()
            .try_recover_unchecked()
            .map_err(|_| ProviderError::SenderRecoveryError)?;
        Ok(Some(block))
    }

    /// Returns the hash of the remote block with the given number.
    pub fn block_hash_by_number(&self, number: u64) -> ProviderResult<Option<B256>> {
        if let Some(hash) = self.hashes.read().get(&number) {
            return Ok(Some(*hash));
        }
        Ok(self.block_by_number(number)?.map(|block| block.hash()))
    }

    /// Returns the number of the remote block with the given hash.
    pub fn block_number_by_hash(&self, hash: B256) -> ProviderResult<Option<u64>> {
        Ok(self.block_by_hash(hash)?.map(|block| block.number()))
    }

    /// Returns the remote header with the given number.
    pub fn header_by_number(&self, number: u64) -> ProviderResult<Option<ForkHeader<F>>> {
        Ok(self.block_by_number(number)?.map(|block| block.header().clone()))
    }

    /// Returns the remote sealed header with the given number.
    pub fn sealed_header(
        &self,
        number: u64,
    ) -> ProviderResult<Option<SealedHeader<ForkHeader<F>>>> {
        Ok(self.block_by_number(number)?.map(|block| block.sealed_header().clone()))
    }

    /// Returns the remote header with the given hash.
    pub fn header_by_hash(&self, hash: B256) -> ProviderResult<Option<ForkHeader<F>>> {
        Ok(self.block_by_hash(hash)?.map(|block| block.header().clone()))
    }

    /// Returns the receipts of the remote block with the given hash.
    pub fn receipts_by_block(
        &self,
        hash: B256,
    ) -> ProviderResult<Option<Arc<Vec<ForkReceipt<F>>>>> {
        if let Some(receipts) = self.receipts.read().get(&hash) {
            return Ok(Some(receipts.clone()));
        }
        if !self.has_remote() {
            return Ok(None);
        }
        let receipts = self.request(move |chain| async move {
            chain.get_block_receipts(hash.into()).await.map_err(Into::into)
        })?;
        let Some(receipts) = receipts else { return Ok(None) };
        // The receipts of the transactions the block leaves out are left out too, so receipts
        // and transactions keep the same positions.
        let receipts = Arc::new(
            receipts
                .into_iter()
                .filter_map(|receipt| F::receipt(receipt).transpose())
                .collect::<ProviderResult<Vec<_>>>()?,
        );
        self.receipts.write().insert(hash, receipts.clone());
        Ok(Some(receipts))
    }

    /// Returns the remote transaction with the given hash and its block metadata.
    pub fn transaction_by_hash_with_meta(
        &self,
        hash: B256,
    ) -> ProviderResult<Option<(ForkTx<F>, TransactionMeta)>> {
        if let Some(dump) = &self.dump
            && let Some((block_hash, index)) = dump.positions.get(&hash).copied()
        {
            let Some(block) = self.block_by_hash(block_hash)? else { return Ok(None) };
            let Some(tx) = block.body().transactions().get(index as usize).cloned() else {
                return Ok(None);
            };
            let meta = TransactionMeta {
                tx_hash: hash,
                index,
                block_hash,
                block_number: block.number(),
                base_fee: block.base_fee_per_gas(),
                excess_blob_gas: block.excess_blob_gas(),
                timestamp: block.timestamp(),
            };
            return Ok(Some((tx, meta)));
        }
        if !self.has_remote() {
            return Ok(None);
        }
        let tx = self.request(move |chain| async move {
            chain.get_transaction_by_hash(hash).await.map_err(Into::into)
        })?;
        let Some(tx) = tx else { return Ok(None) };
        let Some((tx, Some((block_hash, block_number, _)))) = F::transaction(tx)? else {
            return Ok(None);
        };
        if !self.predates_fork(block_number) && block_number != self.header.number() {
            return Ok(None);
        }
        let Some(block) = self.block_by_hash(block_hash)? else { return Ok(None) };
        // The position among the transactions the node keeps, not the remote one.
        let Some(index) =
            block.body().transactions().iter().position(|candidate| *candidate.tx_hash() == hash)
        else {
            return Ok(None);
        };
        let index = index as u64;
        let meta = TransactionMeta {
            tx_hash: hash,
            index,
            block_hash,
            block_number,
            base_fee: block.base_fee_per_gas(),
            excess_blob_gas: block.excess_blob_gas(),
            timestamp: block.timestamp(),
        };
        Ok(Some((tx, meta)))
    }

    /// Returns the remote receipt of the transaction with the given hash.
    pub fn receipt_by_hash(&self, hash: B256) -> ProviderResult<Option<ForkReceipt<F>>> {
        let Some((_, meta)) = self.transaction_by_hash_with_meta(hash)? else { return Ok(None) };
        let Some(receipts) = self.receipts_by_block(meta.block_hash)? else { return Ok(None) };
        Ok(receipts.get(meta.index as usize).cloned())
    }
}

fn spawn_state_backend(
    provider: Arc<RetryProvider>,
    db: BlockchainDb,
    anchor: ForkAnchor,
    by_number: bool,
) -> Result<SharedBackend> {
    let (backend, handler) = if by_number {
        SharedBackend::new_with_anchor_by_number(provider, db, anchor)?
    } else {
        SharedBackend::new_with_anchor(provider, db, anchor)?
    };
    spawn_backend_handler(handler)?;
    Ok(backend)
}

/// Runs a backend handler on its own thread and runtime: the backend blocks the calling thread
/// while it waits, so it must not depend on the node runtime to make progress.
fn spawn_backend_handler<H: Future<Output = ()> + Send + 'static>(handler: H) -> Result<()> {
    std::thread::Builder::new().name("fork-backend".into()).spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build fork backend runtime")
            .block_on(handler)
    })?;
    Ok(())
}

/// Returns the latest remote block that has a hash, walking back at most two blocks.
async fn find_latest_fork_block<P: Provider<alloy_network::AnyNetwork>>(
    provider: &P,
) -> Result<u64> {
    let mut number = provider.get_block_number().await?;
    for _ in 0..2 {
        if let Some(block) = provider.get_block(number.into()).await?
            && !block.header.hash.is_zero()
        {
            break;
        }
        number = number.saturating_sub(1);
    }
    Ok(number)
}

/// State provider that reads locally written keys from the local state and everything else from
/// the remote state.
pub struct ForkStateProvider<F: ForkNetwork = EthereumFork> {
    fork: Arc<ForkBackend<F>>,
    /// The local state and its write index. `None` below the fork block, where the local chain has
    /// no state.
    local: Option<(StateProviderBox, Box<dyn LocalWrites>)>,
    remote: RemoteState,
    /// The block whose state this provider serves.
    block: u64,
}

impl<F: ForkNetwork> Debug for ForkStateProvider<F> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForkStateProvider").field("block", &self.block).finish_non_exhaustive()
    }
}

impl<F: ForkNetwork> ForkStateProvider<F> {
    /// Creates the provider for the state at `block`.
    pub fn new(
        fork: Arc<ForkBackend<F>>,
        local: Option<(StateProviderBox, Box<dyn LocalWrites>)>,
        block: u64,
    ) -> ProviderResult<Self> {
        let remote = fork.state_at(block)?.with_blocking_mode(blocking_mode());
        Ok(Self { fork, local, remote, block })
    }

    fn local(&self) -> ProviderResult<&StateProviderBox> {
        self.local.as_ref().map(|(local, _)| local).ok_or(ProviderError::UnsupportedProvider)
    }

    /// Returns the local state when the local chain wrote the account info of `address`.
    fn local_account(&self, address: &Address) -> ProviderResult<Option<&StateProviderBox>> {
        let Some((local, writes)) = &self.local else { return Ok(None) };
        Ok(writes.account_is_local(address, self.block)?.then_some(local))
    }

    /// Returns the local state when the local chain wrote `slot` of `address`.
    fn local_slot(
        &self,
        address: &Address,
        slot: &StorageKey,
    ) -> ProviderResult<Option<&StateProviderBox>> {
        let Some((local, writes)) = &self.local else { return Ok(None) };
        Ok(writes.slot_is_local(address, slot, self.block)?.then_some(local))
    }
}

impl<F: ForkNetwork> AccountReader for ForkStateProvider<F> {
    fn basic_account(&self, address: &Address) -> ProviderResult<Option<Account>> {
        if let Some(local) = self.local_account(address)? {
            return local.basic_account(address);
        }
        let Some(info) = self.remote.basic(*address)? else {
            return Ok(None);
        };
        if info.is_empty() {
            return Ok(None);
        }
        let account = Account {
            nonce: info.nonce,
            balance: info.balance,
            bytecode_hash: (!info.is_empty_code_hash()).then_some(info.code_hash),
        };
        if self.local.is_some() {
            self.fork.record_account(*address, &info, &account);
        } else {
            self.fork.record_code(&info);
        }
        Ok(Some(account))
    }
}

impl<F: ForkNetwork> BytecodeReader for ForkStateProvider<F> {
    fn bytecode_by_hash(&self, code_hash: &B256) -> ProviderResult<Option<Bytecode>> {
        if let Some((local, _)) = &self.local
            && let Some(code) = local.bytecode_by_hash(code_hash)?
        {
            return Ok(Some(code));
        }
        self.fork.code_by_hash(code_hash)
    }
}

impl<F: ForkNetwork> StateProvider for ForkStateProvider<F> {
    fn storage(
        &self,
        account: Address,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(local) = self.local_slot(&account, &storage_key)? {
            return local.storage(account, storage_key);
        }
        let value = self.remote.storage(account, U256::from_be_bytes(storage_key.0))?;
        if self.local.is_some() {
            self.fork.record_slot(account, storage_key, value);
        }
        Ok(Some(value))
    }

    fn account_code(&self, addr: &Address) -> ProviderResult<Option<Bytecode>> {
        if let Some(local) = self.local_account(addr)? {
            return local.account_code(addr);
        }
        let Some(info) = self.remote.basic(*addr)? else {
            return Ok(None);
        };
        if info.is_empty() {
            return Ok(None);
        }
        let account = Account {
            nonce: info.nonce,
            balance: info.balance,
            bytecode_hash: (!info.is_empty_code_hash()).then_some(info.code_hash),
        };
        if self.local.is_some() {
            self.fork.record_account(*addr, &info, &account);
        }
        Ok(self.fork.record_code(&info))
    }
}

impl<F: ForkNetwork> BlockHashReader for ForkStateProvider<F> {
    fn block_hash(&self, number: u64) -> ProviderResult<Option<B256>> {
        if self.fork.predates_fork(number) || self.local.is_none() {
            return self.fork.block_hash_by_number(number);
        }
        self.local()?.block_hash(number)
    }

    fn canonical_hashes_range(&self, start: u64, end: u64) -> ProviderResult<Vec<B256>> {
        (start..end)
            .map(|number| {
                self.block_hash(number)?.ok_or(ProviderError::HeaderNotFound(number.into()))
            })
            .collect()
    }
}

impl<F: ForkNetwork> StateRootProvider for ForkStateProvider<F> {
    fn state_root(&self, hashed_state: HashedPostState) -> ProviderResult<B256> {
        self.local()?.state_root(hashed_state)
    }

    fn state_root_from_nodes(&self, input: TrieInput) -> ProviderResult<B256> {
        self.local()?.state_root_from_nodes(input)
    }

    fn state_root_with_updates(
        &self,
        hashed_state: HashedPostState,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        self.local()?.state_root_with_updates(hashed_state)
    }

    fn state_root_from_nodes_with_updates(
        &self,
        input: TrieInput,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        self.local()?.state_root_from_nodes_with_updates(input)
    }
}

impl<F: ForkNetwork> StorageRootProvider for ForkStateProvider<F> {
    fn storage_root(
        &self,
        address: Address,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<B256> {
        self.local()?.storage_root(address, hashed_storage)
    }

    fn storage_proof(
        &self,
        address: Address,
        slot: B256,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageProof> {
        self.local()?.storage_proof(address, slot, hashed_storage)
    }

    fn storage_multiproof(
        &self,
        address: Address,
        slots: &[B256],
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageMultiProof> {
        self.local()?.storage_multiproof(address, slots, hashed_storage)
    }
}

impl<F: ForkNetwork> StateProofProvider for ForkStateProvider<F> {
    fn proof(
        &self,
        input: TrieInput,
        address: Address,
        slots: &[B256],
    ) -> ProviderResult<AccountProof> {
        // Below the fork block, the remote endpoint holds the state and its proofs. Above it, an
        // account the local chain has not written still lives in the remote state at the fork
        // block, and only the remote endpoint can prove it; a locally written account is proven
        // against the local trie.
        let Some((local, writes)) = &self.local else {
            return self.fork.account_proof(address, slots, self.block);
        };
        let mut is_local = writes.account_is_local(&address, self.block)?;
        for slot in slots {
            if is_local {
                break;
            }
            is_local = writes.slot_is_local(&address, slot, self.block)?;
        }
        if is_local {
            local.proof(input, address, slots)
        } else {
            self.fork.account_proof(address, slots, self.fork.block_number())
        }
    }

    fn multiproof(
        &self,
        input: TrieInput,
        targets: MultiProofTargets,
    ) -> ProviderResult<MultiProof> {
        self.local()?.multiproof(input, targets)
    }

    fn multiproof_v2(
        &self,
        input: TrieInput,
        targets: MultiProofTargetsV2,
    ) -> ProviderResult<DecodedMultiProofV2> {
        self.local()?.multiproof_v2(input, targets)
    }

    fn witness(
        &self,
        input: TrieInput,
        target: HashedPostState,
        mode: ExecutionWitnessMode,
    ) -> ProviderResult<Vec<Bytes>> {
        self.local()?.witness(input, target, mode)
    }
}

impl<F: ForkNetwork> HashedPostStateProvider for ForkStateProvider<F> {
    fn hashed_post_state(&self, bundle_state: &BundleState) -> ProviderResult<HashedPostState> {
        match &self.local {
            Some((local, _)) => local.hashed_post_state(bundle_state),
            None => Ok(HashedPostState::from_bundle_state::<reth_ethereum::trie::KeccakKeyHasher>(
                &bundle_state.state,
            )),
        }
    }
}
