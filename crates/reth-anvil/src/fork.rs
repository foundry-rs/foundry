//! Forking: chain data and state below the fork block come from a remote endpoint.
//!
//! The local database starts at the fork block, which stands in for genesis: its header is the
//! remote header, so the next local block links to the remote chain. Blocks below the fork block
//! are fetched from the remote endpoint on demand. State reads check whether the local chain has
//! written the account or slot since the fork, through reth's history index and the in-memory
//! blocks; everything else is read from the remote endpoint at the fork block, through foundry's
//! cached fork database.

use crate::{config::NodeConfig, types::ForkChoice};
use alloy_consensus::{BlockHeader, transaction::TransactionMeta};
use alloy_eips::{BlockHashOrNumber, BlockId};
use alloy_network::{Ethereum, Network};
use alloy_primitives::{Address, B256, Bytes, StorageKey, StorageValue, TxNumber, U256, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{Block as RpcBlock, Transaction as RpcTransaction, TransactionReceipt};
use eyre::{Result, WrapErr};
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use foundry_config::Config;
use foundry_evm_core::utils::block_env_from_header;
use foundry_fork_db::{
    BlockchainDb, ForkBlock as ForkAnchor, SharedBackend, backend::BlockingMode,
    cache::BlockchainDbMeta,
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

    /// Converts a remote block into a sealed block.
    fn block(
        response: <Self::Network as Network>::BlockResponse,
    ) -> Result<SealedBlock<<Self::Primitives as NodePrimitives>::Block>, ProviderError>;

    /// Converts a remote receipt.
    fn receipt(
        response: <Self::Network as Network>::ReceiptResponse,
    ) -> Result<<Self::Primitives as NodePrimitives>::Receipt, ProviderError>;

    /// Converts a remote transaction into the signed transaction and its position in the chain,
    /// when it is mined.
    fn transaction(
        response: <Self::Network as Network>::TransactionResponse,
    ) -> Result<(<Self::Primitives as NodePrimitives>::SignedTx, Option<TxPosition>), ProviderError>;
}

/// The block hash, block number, and index of a mined transaction.
pub type TxPosition = (B256, u64, u64);

/// The fork details the RPC namespace reports and changes.
pub trait ForkInfo: Send + Sync + Debug + 'static {
    /// Returns the fork endpoint.
    fn url(&self) -> String;
    /// Returns the chain id of the remote chain.
    fn chain_id(&self) -> u64;
    /// Returns the fork block number.
    fn block_number(&self) -> u64;
    /// Returns the fork block hash.
    fn block_hash(&self) -> B256;
    /// Returns the initial backoff of request retries.
    fn retry_backoff(&self) -> Duration;
    /// Replaces the fork endpoint.
    fn set_rpc_url(&self, url: String) -> Result<()>;
}

impl<F: ForkNetwork> ForkInfo for ForkBackend<F> {
    fn url(&self) -> String {
        Self::url(self)
    }

    fn chain_id(&self) -> u64 {
        Self::chain_id(self)
    }

    fn block_number(&self) -> u64 {
        Self::block_number(self)
    }

    fn block_hash(&self) -> B256 {
        Self::block_hash(self)
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
    type Network = Ethereum;
    type Primitives = EthPrimitives;

    fn block(response: RpcBlock) -> Result<SealedBlock<Block>, ProviderError> {
        let hash = response.header.hash;
        let block = response.into_consensus().map_transactions(|tx| tx.into_inner().into());
        Ok(SealedBlock::new_unchecked(block, hash))
    }

    fn receipt(response: TransactionReceipt) -> Result<Receipt, ProviderError> {
        Ok(Receipt {
            tx_type: response.inner.tx_type(),
            success: response.inner.status(),
            cumulative_gas_used: response.inner.cumulative_gas_used(),
            logs: response.inner.logs().iter().map(|log| log.inner.clone()).collect(),
        })
    }

    fn transaction(
        response: RpcTransaction,
    ) -> Result<(TransactionSigned, Option<TxPosition>), ProviderError> {
        let position =
            match (response.block_hash, response.block_number, response.transaction_index) {
                (Some(hash), Some(number), Some(index)) => Some((hash, number, index)),
                _ => None,
            };
        Ok((response.into_inner().into(), position))
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
        let settings = config.fork_settings();
        let url = settings.urls.first().cloned().ok_or_else(|| eyre::eyre!("no fork url"))?;
        // The node runtime drives this provider. The backends get their own providers below, so
        // their connections live on the backend thread: a read may block the node runtime while
        // it waits for the backend, and a connection driven by the blocked runtime would stall.
        let provider = settings.provider::<alloy_network::AnyNetwork>(&url)?;
        let chain = settings.provider::<F::Network>(&url)?;

        let chain_id = match config.fork_chain_id {
            Some(chain_id) => chain_id,
            None => provider.get_chain_id().await.wrap_err("failed to fetch network chain ID")?,
        };

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
                let (_, position) = F::transaction(tx)?;
                let Some((_, number, index)) = position else {
                    eyre::bail!("transaction {hash} is not mined yet");
                };
                if number == 0 {
                    eyre::bail!("transaction {hash} is in the genesis block");
                }
                replay_target = Some((number, index));
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
        let block = F::block(block)?;
        let hash = block.hash();
        let header = block.sealed_header().clone();
        let gas_price =
            provider.get_gas_price().await.unwrap_or(crate::config::INITIAL_BASE_FEE as u128);
        let replay = match replay_target {
            Some((number, index)) => {
                let block = chain
                    .get_block_by_number(number.into())
                    .full()
                    .await?
                    .ok_or_else(|| eyre::eyre!("failed to get block {number} from the fork"))?;
                let block = F::block(block)?;
                let transactions =
                    block.body().transactions().iter().take(index as usize + 1).cloned().collect();
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
                .with_fork_identity(hash, settings.source_id());
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
            }),
            genesis_accounts,
        ))
    }

    /// Returns the fork endpoint.
    pub fn url(&self) -> String {
        self.url.read().clone()
    }

    /// Takes the transactions to replay at startup, if the fork is at a transaction hash.
    pub fn take_replay(&self) -> Option<ForkReplay<F>> {
        self.replay.write().take()
    }

    /// Returns the chain id of the remote chain.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
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
            .with_fork_identity(self.header.hash(), self.settings.source_id());
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
    pub fn state_at(&self, number: u64) -> ProviderResult<SharedBackend> {
        if number >= self.header.number() {
            return Ok(self.state.read().clone());
        }
        if let Some(backend) = self.history.read().get(&number) {
            return Ok(backend.clone());
        }
        let header = self
            .header_by_number(number)?
            .ok_or(ProviderError::HeaderNotFound(BlockHashOrNumber::Number(number)))?;
        let url = self.url();
        let meta = BlockchainDbMeta::new(block_env_from_header::<BlockEnv>(&header), url.clone());
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
        Ok(backend)
    }

    /// Returns the remote bytecode with the given hash, if a remote account read fetched it.
    pub fn code_by_hash(&self, hash: &B256) -> Option<Bytecode> {
        self.codes.read().get(hash).cloned()
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
        let future = request(self.chain.read().clone());
        let mut state = self.state.read().with_blocking_mode(blocking_mode());
        state.do_any_request(future).map_err(ProviderError::other)
    }

    fn cache_block(
        &self,
        response: <F::Network as Network>::BlockResponse,
    ) -> ProviderResult<Arc<SealedBlock<ForkBlock<F>>>> {
        let block = Arc::new(F::block(response)?);
        let hash = block.hash();
        self.hashes.write().insert(block.number(), hash);
        self.blocks.write().insert(hash, block.clone());
        Ok(block)
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
        let block = self.request(move |chain| async move {
            chain.get_block_by_hash(hash).full().await.map_err(Into::into)
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
        let block = self.request(move |chain| async move {
            chain.get_block_by_number(number.into()).full().await.map_err(Into::into)
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
        let receipts = self.request(move |chain| async move {
            chain.get_block_receipts(hash.into()).await.map_err(Into::into)
        })?;
        let Some(receipts) = receipts else { return Ok(None) };
        let receipts =
            Arc::new(receipts.into_iter().map(F::receipt).collect::<ProviderResult<Vec<_>>>()?);
        self.receipts.write().insert(hash, receipts.clone());
        Ok(Some(receipts))
    }

    /// Returns the remote transaction with the given hash and its block metadata.
    pub fn transaction_by_hash_with_meta(
        &self,
        hash: B256,
    ) -> ProviderResult<Option<(ForkTx<F>, TransactionMeta)>> {
        let tx = self.request(move |chain| async move {
            chain.get_transaction_by_hash(hash).await.map_err(Into::into)
        })?;
        let Some(tx) = tx else { return Ok(None) };
        let (tx, Some((block_hash, block_number, index))) = F::transaction(tx)? else {
            return Ok(None);
        };
        if !self.predates_fork(block_number) && block_number != self.header.number() {
            return Ok(None);
        }
        let Some(block) = self.block_by_hash(block_hash)? else { return Ok(None) };
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
    // The handler gets its own thread and runtime: the backend blocks the calling thread while it
    // waits, so it must not depend on the node runtime to make progress.
    std::thread::Builder::new().name("fork-backend".into()).spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build fork backend runtime")
            .block_on(handler)
    })?;
    Ok(backend)
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
    remote: SharedBackend,
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
        let Some(info) = self.remote.basic_ref(*address).map_err(ProviderError::other)? else {
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
        Ok(self.fork.code_by_hash(code_hash))
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
        let value = self
            .remote
            .storage_ref(account, U256::from_be_bytes(storage_key.0))
            .map_err(ProviderError::other)?;
        if self.local.is_some() {
            self.fork.record_slot(account, storage_key, value);
        }
        Ok(Some(value))
    }

    fn account_code(&self, addr: &Address) -> ProviderResult<Option<Bytecode>> {
        if let Some(local) = self.local_account(addr)? {
            return local.account_code(addr);
        }
        let Some(info) = self.remote.basic_ref(*addr).map_err(ProviderError::other)? else {
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
