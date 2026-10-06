//! Forking: chain data and state below the fork block come from a remote endpoint.
//!
//! The local database starts at the fork block, which stands in for genesis: its header is the
//! remote header, so the next local block links to the remote chain. Blocks below the fork block
//! are fetched from the remote endpoint on demand. State reads check whether the local chain has
//! written the account or slot since the fork, through reth's history index and the in-memory
//! blocks; everything else is read from the remote endpoint at the fork block, through foundry's
//! cached fork database.

use crate::{config::NodeConfig, types::ForkChoice};
use alloy_consensus::{BlockHeader, Header, transaction::TransactionMeta};
use alloy_eips::{BlockHashOrNumber, BlockId};
use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, Bytes, StorageKey, StorageValue, TxNumber, U256, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types_eth::{Block as RpcBlock, TransactionReceipt};
use eyre::{Result, WrapErr};
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use foundry_config::Config;
use foundry_evm_core::utils::block_env_from_header;
use foundry_fork_db::{
    BlockchainDb, ForkBlock, SharedBackend, backend::BlockingMode, cache::BlockchainDbMeta,
};
use parking_lot::RwLock;
use reth_ethereum::{
    Block, Receipt, TransactionSigned,
    primitives::{Account, Bytecode, RecoveredBlock, SealedBlock, SealedHeader},
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

/// The remote side of a fork.
pub struct ForkBackend {
    settings: ForkSettings,
    url: RwLock<String>,
    chain_id: u64,
    header: SealedHeader,
    gas_price: u128,
    /// The remote state at the fork block.
    state: RwLock<SharedBackend>,
    /// The remote state at blocks below the fork block, by block number.
    history: RwLock<HashMap<u64, SharedBackend>>,
    /// The chain reader for blocks below the fork block.
    chain: RwLock<RetryProvider<Ethereum>>,
    /// Remote bytecodes by hash.
    codes: RwLock<HashMap<B256, Bytecode>>,
    reads: RwLock<RemoteReads>,
    blocks: RwLock<HashMap<B256, Arc<SealedBlock<Block>>>>,
    hashes: RwLock<HashMap<u64, B256>>,
    receipts: RwLock<HashMap<B256, Arc<Vec<Receipt>>>>,
}

impl Debug for ForkBackend {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForkBackend")
            .field("url", &*self.url.read())
            .field("chain_id", &self.chain_id)
            .field("block_number", &self.header.number)
            .field("block_hash", &self.header.hash())
            .finish_non_exhaustive()
    }
}

impl ForkBackend {
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
        let chain = settings.provider::<Ethereum>(&url)?;

        let chain_id = match config.fork_chain_id {
            Some(chain_id) => chain_id,
            None => provider.get_chain_id().await.wrap_err("failed to fetch network chain ID")?,
        };

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
            Some(ForkChoice::Transaction(_)) => {
                eyre::bail!("forking at a transaction hash is not supported yet")
            }
            None => find_latest_fork_block(&provider)
                .await
                .wrap_err("failed to get fork block number")?,
        };

        let block = chain.get_block_by_number(block_number.into()).await?;
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
        let hash = block.header.hash;
        let header = SealedHeader::new(block.header.inner.clone(), hash);
        let gas_price =
            provider.get_gas_price().await.unwrap_or(crate::config::INITIAL_BASE_FEE as u128);

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
            ForkBlock::new(block_number, hash),
            settings.state_by_number,
        )?;
        let chain = settings.provider::<Ethereum>(&url)?;

        Ok((
            Arc::new(Self {
                settings,
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

    /// Returns the chain id of the remote chain.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the fork block number.
    pub fn block_number(&self) -> u64 {
        self.header.number
    }

    /// Returns the fork block hash.
    pub fn block_hash(&self) -> B256 {
        self.header.hash()
    }

    /// Returns the fork block header.
    pub const fn header(&self) -> &SealedHeader {
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
        number < self.header.number
    }

    /// Replaces the fork endpoint. The state fetched so far stays cached.
    pub fn set_rpc_url(&self, url: String) -> Result<()> {
        let provider = self.settings.provider::<alloy_network::AnyNetwork>(&url)?;
        let chain = self.settings.provider::<Ethereum>(&url)?;
        let db = {
            let state = self.state.read();
            let meta = BlockchainDbMeta::new(
                block_env_from_header::<BlockEnv>(self.header.header()),
                url.clone(),
            )
            .with_fork_identity(self.header.hash(), self.settings.source_id());
            let db = BlockchainDb::new(
                meta,
                self.settings.cache_path(self.chain_id, self.header.number, &url),
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
            ForkBlock::new(self.header.number, self.header.hash()),
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
        if number >= self.header.number {
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
    fn request<T, F>(&self, request: impl FnOnce(RetryProvider<Ethereum>) -> F) -> ProviderResult<T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Debug + Send + 'static,
    {
        let future = request(self.chain.read().clone());
        let mut state = self.state.read().with_blocking_mode(blocking_mode());
        state.do_any_request(future).map_err(ProviderError::other)
    }

    fn cache_block(&self, block: RpcBlock) -> Arc<SealedBlock<Block>> {
        let hash = block.header.hash;
        let block = block.into_consensus().map_transactions(|tx| tx.into_inner().into());
        let block = Arc::new(SealedBlock::new_unchecked(block, hash));
        self.hashes.write().insert(block.number(), hash);
        self.blocks.write().insert(hash, block.clone());
        block
    }

    /// Returns the remote block with the given hash.
    pub fn block_by_hash(&self, hash: B256) -> ProviderResult<Option<Arc<SealedBlock<Block>>>> {
        if let Some(block) = self.blocks.read().get(&hash) {
            return Ok(Some(block.clone()));
        }
        let block = self.request(move |chain| async move {
            chain.get_block_by_hash(hash).full().await.map_err(Into::into)
        })?;
        Ok(block.map(|block| self.cache_block(block)))
    }

    /// Returns the remote block with the given number.
    pub fn block_by_number(&self, number: u64) -> ProviderResult<Option<Arc<SealedBlock<Block>>>> {
        if let Some(hash) = self.hashes.read().get(&number)
            && let Some(block) = self.blocks.read().get(hash)
        {
            return Ok(Some(block.clone()));
        }
        let block = self.request(move |chain| async move {
            chain.get_block_by_number(number.into()).full().await.map_err(Into::into)
        })?;
        Ok(block.map(|block| self.cache_block(block)))
    }

    /// Returns the remote block for the given id.
    pub fn block(&self, id: BlockHashOrNumber) -> ProviderResult<Option<Arc<SealedBlock<Block>>>> {
        match id {
            BlockHashOrNumber::Hash(hash) => self.block_by_hash(hash),
            BlockHashOrNumber::Number(number) => self.block_by_number(number),
        }
    }

    /// Returns the remote block for the given id with recovered senders.
    pub fn recovered_block(
        &self,
        id: BlockHashOrNumber,
    ) -> ProviderResult<Option<RecoveredBlock<Block>>> {
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
    pub fn header_by_number(&self, number: u64) -> ProviderResult<Option<Header>> {
        Ok(self.block_by_number(number)?.map(|block| block.header().clone()))
    }

    /// Returns the remote sealed header with the given number.
    pub fn sealed_header(&self, number: u64) -> ProviderResult<Option<SealedHeader>> {
        Ok(self.block_by_number(number)?.map(|block| block.sealed_header().clone()))
    }

    /// Returns the remote header with the given hash.
    pub fn header_by_hash(&self, hash: B256) -> ProviderResult<Option<Header>> {
        Ok(self.block_by_hash(hash)?.map(|block| block.header().clone()))
    }

    /// Returns the receipts of the remote block with the given hash.
    pub fn receipts_by_block(&self, hash: B256) -> ProviderResult<Option<Arc<Vec<Receipt>>>> {
        if let Some(receipts) = self.receipts.read().get(&hash) {
            return Ok(Some(receipts.clone()));
        }
        let receipts = self.request(move |chain| async move {
            chain.get_block_receipts(hash.into()).await.map_err(Into::into)
        })?;
        let Some(receipts) = receipts else { return Ok(None) };
        let receipts = Arc::new(receipts.into_iter().map(convert_receipt).collect::<Vec<_>>());
        self.receipts.write().insert(hash, receipts.clone());
        Ok(Some(receipts))
    }

    /// Returns the remote transaction with the given hash and its block metadata.
    pub fn transaction_by_hash_with_meta(
        &self,
        hash: B256,
    ) -> ProviderResult<Option<(TransactionSigned, TransactionMeta)>> {
        let tx = self.request(move |chain| async move {
            chain.get_transaction_by_hash(hash).await.map_err(Into::into)
        })?;
        let Some(tx) = tx else { return Ok(None) };
        let (Some(block_hash), Some(block_number), Some(index)) =
            (tx.block_hash, tx.block_number, tx.transaction_index)
        else {
            return Ok(None);
        };
        if !self.predates_fork(block_number) && block_number != self.header.number {
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
        Ok(Some((tx.into_inner().into(), meta)))
    }

    /// Returns the remote receipt of the transaction with the given hash.
    pub fn receipt_by_hash(&self, hash: B256) -> ProviderResult<Option<Receipt>> {
        let Some((_, meta)) = self.transaction_by_hash_with_meta(hash)? else { return Ok(None) };
        let Some(receipts) = self.receipts_by_block(meta.block_hash)? else { return Ok(None) };
        Ok(receipts.get(meta.index as usize).cloned())
    }
}

fn spawn_state_backend(
    provider: Arc<RetryProvider>,
    db: BlockchainDb,
    anchor: ForkBlock,
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

fn convert_receipt(receipt: TransactionReceipt) -> Receipt {
    Receipt {
        tx_type: receipt.inner.tx_type(),
        success: receipt.inner.status(),
        cumulative_gas_used: receipt.inner.cumulative_gas_used(),
        logs: receipt.inner.logs().iter().map(|log| log.inner.clone()).collect(),
    }
}

/// State provider that reads locally written keys from the local state and everything else from
/// the remote state.
pub struct ForkStateProvider {
    fork: Arc<ForkBackend>,
    /// The local state and its write index. `None` below the fork block, where the local chain has
    /// no state.
    local: Option<(StateProviderBox, Box<dyn LocalWrites>)>,
    remote: SharedBackend,
    /// The block whose state this provider serves.
    block: u64,
}

impl Debug for ForkStateProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForkStateProvider").field("block", &self.block).finish_non_exhaustive()
    }
}

impl ForkStateProvider {
    /// Creates the provider for the state at `block`.
    pub fn new(
        fork: Arc<ForkBackend>,
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

impl AccountReader for ForkStateProvider {
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

impl BytecodeReader for ForkStateProvider {
    fn bytecode_by_hash(&self, code_hash: &B256) -> ProviderResult<Option<Bytecode>> {
        if let Some((local, _)) = &self.local
            && let Some(code) = local.bytecode_by_hash(code_hash)?
        {
            return Ok(Some(code));
        }
        Ok(self.fork.code_by_hash(code_hash))
    }
}

impl StateProvider for ForkStateProvider {
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

impl BlockHashReader for ForkStateProvider {
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

impl StateRootProvider for ForkStateProvider {
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

impl StorageRootProvider for ForkStateProvider {
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

impl StateProofProvider for ForkStateProvider {
    fn proof(
        &self,
        input: TrieInput,
        address: Address,
        slots: &[B256],
    ) -> ProviderResult<AccountProof> {
        self.local()?.proof(input, address, slots)
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

impl HashedPostStateProvider for ForkStateProvider {
    fn hashed_post_state(&self, bundle_state: &BundleState) -> ProviderResult<HashedPostState> {
        match &self.local {
            Some((local, _)) => local.hashed_post_state(bundle_state),
            None => Ok(HashedPostState::from_bundle_state::<reth_ethereum::trie::KeccakKeyHasher>(
                &bundle_state.state,
            )),
        }
    }
}
