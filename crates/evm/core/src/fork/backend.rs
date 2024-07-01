//! Smart caching and deduplication of requests when using a forking provider
use crate::{
    backend::{DatabaseError, DatabaseResult},
    fork::{cache::FlushJsonBlockCacheDB, BlockchainDb},
};
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_provider::{network::AnyNetwork, Provider};
use alloy_rpc_types::{Block, BlockId, Transaction};
use alloy_serde::WithOtherFields;
use alloy_transport::Transport;
use eyre::WrapErr;
use foundry_common::NON_ARCHIVE_NODE_WARNING;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::Stream,
    task::{Context, Poll},
    Future, FutureExt,
};
use revm::{
    db::DatabaseRef,
    primitives::{AccountInfo, Bytecode, KECCAK_EMPTY},
};
use rustc_hash::FxHashMap;
use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    future::IntoFuture,
    marker::PhantomData,
    pin::Pin,
    sync::{
        mpsc::{channel as oneshot_channel, Sender as OneshotSender},
        Arc,
    },
};

// Various future/request type aliases

type AccountFuture<Err> =
    Pin<Box<dyn Future<Output = (Result<(U256, u64, Bytes), Err>, Address)> + Send>>;
type StorageFuture<Err> = Pin<Box<dyn Future<Output = (Result<U256, Err>, Address, U256)> + Send>>;
type BlockHashFuture<Err> = Pin<Box<dyn Future<Output = (Result<B256, Err>, u64)> + Send>>;
type FullBlockFuture<Err> =
    Pin<Box<dyn Future<Output = (FullBlockSender, Result<Option<Block>, Err>, BlockId)> + Send>>;
type TransactionFuture<Err> = Pin<
    Box<
        dyn Future<Output = (TransactionSender, Result<WithOtherFields<Transaction>, Err>, B256)>
            + Send,
    >,
>;

type AccountInfoSender = OneshotSender<DatabaseResult<AccountInfo>>;
type StorageSender = OneshotSender<DatabaseResult<U256>>;
type BlockHashSender = OneshotSender<DatabaseResult<B256>>;
type FullBlockSender = OneshotSender<DatabaseResult<Block>>;
type TransactionSender = OneshotSender<DatabaseResult<WithOtherFields<Transaction>>>;

/// Request variants that are executed by the provider
enum ProviderRequest<Err> {
    Account(AccountFuture<Err>),
    Storage(StorageFuture<Err>),
    BlockHash(BlockHashFuture<Err>),
    FullBlock(FullBlockFuture<Err>),
    Transaction(TransactionFuture<Err>),
}

/// The Request type the Backend listens for
#[derive(Debug)]
enum BackendRequest {
    /// Fetch the account info
    Basic(Address, AccountInfoSender),
    /// Fetch a storage slot
    Storage(Address, U256, StorageSender),
    /// Fetch a block hash
    BlockHash(u64, BlockHashSender),
    /// Fetch an entire block with transactions
    FullBlock(BlockId, FullBlockSender),
    /// Fetch a transaction
    Transaction(B256, TransactionSender),
    /// Sets the pinned block to fetch data from
    SetPinnedBlock(BlockId),
}

/// Handles an internal provider and listens for requests.
///
/// This handler will remain active as long as it is reachable (request channel still open) and
/// requests are in progress.
#[must_use = "futures do nothing unless polled"]
pub struct BackendHandler<T, P> {
    provider: P,
    transport: PhantomData<T>,
    /// Stores all the data.
    db: BlockchainDb,
    /// Requests currently in progress
    pending_requests: Vec<ProviderRequest<eyre::Report>>,
    /// Listeners that wait for a `get_account` related response
    account_requests: HashMap<Address, Vec<AccountInfoSender>>,
    /// Listeners that wait for a `get_storage_at` response
    storage_requests: HashMap<(Address, U256), Vec<StorageSender>>,
    /// Listeners that wait for a `get_block` response
    block_requests: FxHashMap<u64, Vec<BlockHashSender>>,
    /// Incoming commands.
    incoming: Receiver<BackendRequest>,
    /// unprocessed queued requests
    queued_requests: VecDeque<BackendRequest>,
    /// The block to fetch data from.
    // This is an `Option` so that we can have less code churn in the functions below
    block_id: Option<BlockId>,
}

impl<T, P> BackendHandler<T, P>
where
    T: Transport + Clone,
    P: Provider<T, AnyNetwork> + Clone + Unpin + 'static,
{
    fn new(
        provider: P,
        db: BlockchainDb,
        rx: Receiver<BackendRequest>,
        block_id: Option<BlockId>,
    ) -> Self {
        Self {
            provider,
            db,
            pending_requests: Default::default(),
            account_requests: Default::default(),
            storage_requests: Default::default(),
            block_requests: Default::default(),
            queued_requests: Default::default(),
            incoming: rx,
            block_id,
            transport: PhantomData,
        }
    }

    /// handle the request in queue in the future.
    ///
    /// We always check:
    ///  1. if the requested value is already stored in the cache, then answer the sender
    ///  2. otherwise, fetch it via the provider but check if a request for that value is already in
    ///     progress (e.g. another Sender just requested the same account)
    fn on_request(&mut self, req: BackendRequest) {
        match req {
            BackendRequest::Basic(addr, sender) => {
                trace!(target: "backendhandler", "received request basic address={:?}", addr);
                let acc = self.db.accounts().read().get(&addr).cloned();
                if let Some(basic) = acc {
                    let _ = sender.send(Ok(basic));
                } else {
                    self.request_account(addr, sender);
                }
            }
            BackendRequest::BlockHash(number, sender) => {
                let hash = self.db.block_hashes().read().get(&U256::from(number)).cloned();
                if let Some(hash) = hash {
                    let _ = sender.send(Ok(hash));
                } else {
                    self.request_hash(number, sender);
                }
            }
            BackendRequest::FullBlock(number, sender) => {
                self.request_full_block(number, sender);
            }
            BackendRequest::Transaction(tx, sender) => {
                self.request_transaction(tx, sender);
            }
            BackendRequest::Storage(addr, idx, sender) => {
                // account is already stored in the cache
                let value =
                    self.db.storage().read().get(&addr).and_then(|acc| acc.get(&idx).copied());
                if let Some(value) = value {
                    let _ = sender.send(Ok(value));
                } else {
                    // account present but not storage -> fetch storage
                    self.request_account_storage(addr, idx, sender);
                }
            }
            BackendRequest::SetPinnedBlock(block_id) => {
                self.block_id = Some(block_id);
            }
        }
    }

    /// process a request for account's storage
    fn request_account_storage(&mut self, address: Address, idx: U256, listener: StorageSender) {
        match self.storage_requests.entry((address, idx)) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(listener);
            }
            Entry::Vacant(entry) => {
                trace!(target: "backendhandler", %address, %idx, "preparing storage request");
                entry.insert(vec![listener]);
                let provider = self.provider.clone();
                let block_id = self.block_id.unwrap_or_default();
                let fut = Box::pin(async move {
                    let storage = provider
                        .get_storage_at(address, idx)
                        .block_id(block_id)
                        .await
                        .map_err(Into::into);
                    (storage, address, idx)
                });
                self.pending_requests.push(ProviderRequest::Storage(fut));
            }
        }
    }

    /// returns the future that fetches the account data
    fn get_account_req(&self, address: Address) -> ProviderRequest<eyre::Report> {
        trace!(target: "backendhandler", "preparing account request, address={:?}", address);
        let provider = self.provider.clone();
        let block_id = self.block_id.unwrap_or_default();
        let fut = Box::pin(async move {
            let balance = provider.get_balance(address).block_id(block_id).into_future();
            let nonce = provider.get_transaction_count(address).block_id(block_id).into_future();
            let code = provider.get_code_at(address).block_id(block_id).into_future();
            let resp = tokio::try_join!(balance, nonce, code).map_err(Into::into);
            (resp, address)
        });
        ProviderRequest::Account(fut)
    }

    /// process a request for an account
    fn request_account(&mut self, address: Address, listener: AccountInfoSender) {
        match self.account_requests.entry(address) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(listener);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![listener]);
                self.pending_requests.push(self.get_account_req(address));
            }
        }
    }

    /// process a request for an entire block
    fn request_full_block(&mut self, number: BlockId, sender: FullBlockSender) {
        let provider = self.provider.clone();
        let fut = Box::pin(async move {
            let block = provider
                .get_block(number, true.into())
                .await
                .wrap_err("could not fetch block {number:?}");
            (sender, block, number)
        });

        self.pending_requests.push(ProviderRequest::FullBlock(fut));
    }

    /// process a request for a transactions
    fn request_transaction(&mut self, tx: B256, sender: TransactionSender) {
        let provider = self.provider.clone();
        let fut = Box::pin(async move {
            let block = provider
                .get_transaction_by_hash(tx)
                .await
                .wrap_err_with(|| format!("could not get transaction {tx}"))
                .and_then(|maybe| {
                    maybe.ok_or_else(|| eyre::eyre!("could not get transaction {tx}"))
                });
            (sender, block, tx)
        });

        self.pending_requests.push(ProviderRequest::Transaction(fut));
    }

    /// process a request for a block hash
    fn request_hash(&mut self, number: u64, listener: BlockHashSender) {
        match self.block_requests.entry(number) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(listener);
            }
            Entry::Vacant(entry) => {
                trace!(target: "backendhandler", number, "preparing block hash request");
                entry.insert(vec![listener]);
                let provider = self.provider.clone();
                let fut = Box::pin(async move {
                    let block = provider
                        .get_block_by_number(number.into(), false)
                        .await
                        .wrap_err("failed to get block");

                    let block_hash = match block {
                        Ok(Some(block)) => Ok(block
                            .header
                            .hash
                            .expect("empty block hash on mined block, this should never happen")),
                        Ok(None) => {
                            warn!(target: "backendhandler", ?number, "block not found");
                            // if no block was returned then the block does not exist, in which case
                            // we return empty hash
                            Ok(KECCAK_EMPTY)
                        }
                        Err(err) => {
                            error!(target: "backendhandler", %err, ?number, "failed to get block");
                            Err(err)
                        }
                    };
                    (block_hash, number)
                });
                self.pending_requests.push(ProviderRequest::BlockHash(fut));
            }
        }
    }
}

impl<T, P> Future for BackendHandler<T, P>
where
    T: Transport + Clone + Unpin,
    P: Provider<T, AnyNetwork> + Clone + Unpin + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        loop {
            // Drain queued requests first.
            while let Some(req) = pin.queued_requests.pop_front() {
                pin.on_request(req)
            }

            // receive new requests to delegate to the underlying provider
            loop {
                match Pin::new(&mut pin.incoming).poll_next(cx) {
                    Poll::Ready(Some(req)) => {
                        pin.queued_requests.push_back(req);
                    }
                    Poll::Ready(None) => {
                        trace!(target: "backendhandler", "last sender dropped, ready to drop (&flush cache)");
                        return Poll::Ready(());
                    }
                    Poll::Pending => break,
                }
            }

            // poll all requests in progress
            for n in (0..pin.pending_requests.len()).rev() {
                let mut request = pin.pending_requests.swap_remove(n);
                match &mut request {
                    ProviderRequest::Account(fut) => {
                        if let Poll::Ready((resp, addr)) = fut.poll_unpin(cx) {
                            // get the response
                            let (balance, nonce, code) = match resp {
                                Ok(res) => res,
                                Err(err) => {
                                    let err = Arc::new(err);
                                    if let Some(listeners) = pin.account_requests.remove(&addr) {
                                        listeners.into_iter().for_each(|l| {
                                            let _ = l.send(Err(DatabaseError::GetAccount(
                                                addr,
                                                Arc::clone(&err),
                                            )));
                                        })
                                    }
                                    continue;
                                }
                            };

                            // convert it to revm-style types
                            let (code, code_hash) = if !code.is_empty() {
                                (code.clone(), keccak256(&code))
                            } else {
                                (Bytes::default(), KECCAK_EMPTY)
                            };

                            // update the cache
                            let acc = AccountInfo {
                                nonce,
                                balance,
                                code: Some(Bytecode::new_raw(code)),
                                code_hash,
                            };
                            pin.db.accounts().write().insert(addr, acc.clone());

                            // notify all listeners
                            if let Some(listeners) = pin.account_requests.remove(&addr) {
                                listeners.into_iter().for_each(|l| {
                                    let _ = l.send(Ok(acc.clone()));
                                })
                            }
                            continue;
                        }
                    }
                    ProviderRequest::Storage(fut) => {
                        if let Poll::Ready((resp, addr, idx)) = fut.poll_unpin(cx) {
                            let value = match resp {
                                Ok(value) => value,
                                Err(err) => {
                                    // notify all listeners
                                    let err = Arc::new(err);
                                    if let Some(listeners) =
                                        pin.storage_requests.remove(&(addr, idx))
                                    {
                                        listeners.into_iter().for_each(|l| {
                                            let _ = l.send(Err(DatabaseError::GetStorage(
                                                addr,
                                                idx,
                                                Arc::clone(&err),
                                            )));
                                        })
                                    }
                                    continue;
                                }
                            };

                            // update the cache
                            pin.db.storage().write().entry(addr).or_default().insert(idx, value);

                            // notify all listeners
                            if let Some(listeners) = pin.storage_requests.remove(&(addr, idx)) {
                                listeners.into_iter().for_each(|l| {
                                    let _ = l.send(Ok(value));
                                })
                            }
                            continue;
                        }
                    }
                    ProviderRequest::BlockHash(fut) => {
                        if let Poll::Ready((block_hash, number)) = fut.poll_unpin(cx) {
                            let value = match block_hash {
                                Ok(value) => value,
                                Err(err) => {
                                    let err = Arc::new(err);
                                    // notify all listeners
                                    if let Some(listeners) = pin.block_requests.remove(&number) {
                                        listeners.into_iter().for_each(|l| {
                                            let _ = l.send(Err(DatabaseError::GetBlockHash(
                                                number,
                                                Arc::clone(&err),
                                            )));
                                        })
                                    }
                                    continue;
                                }
                            };

                            // update the cache
                            pin.db.block_hashes().write().insert(U256::from(number), value);

                            // notify all listeners
                            if let Some(listeners) = pin.block_requests.remove(&number) {
                                listeners.into_iter().for_each(|l| {
                                    let _ = l.send(Ok(value));
                                })
                            }
                            continue;
                        }
                    }
                    ProviderRequest::FullBlock(fut) => {
                        if let Poll::Ready((sender, resp, number)) = fut.poll_unpin(cx) {
                            let msg = match resp {
                                Ok(Some(block)) => Ok(block),
                                Ok(None) => Err(DatabaseError::BlockNotFound(number)),
                                Err(err) => {
                                    let err = Arc::new(err);
                                    Err(DatabaseError::GetFullBlock(number, err))
                                }
                            };
                            let _ = sender.send(msg);
                            continue;
                        }
                    }
                    ProviderRequest::Transaction(fut) => {
                        if let Poll::Ready((sender, tx, tx_hash)) = fut.poll_unpin(cx) {
                            let msg = match tx {
                                Ok(tx) => Ok(tx),
                                Err(err) => {
                                    let err = Arc::new(err);
                                    Err(DatabaseError::GetTransaction(tx_hash, err))
                                }
                            };
                            let _ = sender.send(msg);
                            continue;
                        }
                    }
                }
                // not ready, insert and poll again
                pin.pending_requests.push(request);
            }

            // If no new requests have been queued, break to
            // be polled again later.
            if pin.queued_requests.is_empty() {
                return Poll::Pending;
            }
        }
    }
}

/// A cloneable backend type that shares access to the backend data with all its clones.
///
/// This backend type is connected to the `BackendHandler` via a mpsc channel. The `BackendHandler`
/// is spawned on a tokio task and listens for incoming commands on the receiver half of the
/// channel. A `SharedBackend` holds a sender for that channel, which is `Clone`, so there can be
/// multiple `SharedBackend`s communicating with the same `BackendHandler`, hence this `Backend`
/// type is thread safe.
///
/// All `Backend` trait functions are delegated as a `BackendRequest` via the channel to the
/// `BackendHandler`. All `BackendRequest` variants include a sender half of an additional channel
/// that is used by the `BackendHandler` to send the result of an executed `BackendRequest` back to
/// `SharedBackend`.
///
/// The `BackendHandler` holds a `Provider` to look up missing accounts or storage slots
/// from remote (e.g. infura). It detects duplicate requests from multiple `SharedBackend`s and
/// bundles them together, so that always only one provider request is executed. For example, there
/// are two `SharedBackend`s, `A` and `B`, both request the basic account info of account
/// `0xasd9sa7d...` at the same time. After the `BackendHandler` receives the request from `A`, it
/// sends a new provider request to the provider's endpoint, then it reads the identical request
/// from `B` and simply adds it as an additional listener for the request already in progress,
/// instead of sending another one. So that after the provider returns the response all listeners
/// (`A` and `B`) get notified.
// **Note**: the implementation makes use of [tokio::task::block_in_place()] when interacting with
// the underlying [BackendHandler] which runs on a separate spawned tokio task.
// [tokio::task::block_in_place()]
// > Runs the provided blocking function on the current thread without blocking the executor.
// This prevents issues (hangs) we ran into were the [SharedBackend] itself is called from a spawned
// task.
#[derive(Clone, Debug)]
pub struct SharedBackend {
    /// channel used for sending commands related to database operations
    backend: Sender<BackendRequest>,
    /// Ensures that the underlying cache gets flushed once the last `SharedBackend` is dropped.
    ///
    /// There is only one instance of the type, so as soon as the last `SharedBackend` is deleted,
    /// `FlushJsonBlockCacheDB` is also deleted and the cache is flushed.
    cache: Arc<FlushJsonBlockCacheDB>,
}

impl SharedBackend {
    /// _Spawns_ a new `BackendHandler` on a `tokio::task` that listens for requests from any
    /// `SharedBackend`. Missing values get inserted in the `db`.
    ///
    /// The spawned `BackendHandler` finishes once the last `SharedBackend` connected to it is
    /// dropped.
    ///
    /// NOTE: this should be called with `Arc<Provider>`
    pub async fn spawn_backend<T, P>(
        provider: P,
        db: BlockchainDb,
        pin_block: Option<BlockId>,
    ) -> Self
    where
        T: Transport + Clone + Unpin,
        P: Provider<T, AnyNetwork> + Unpin + 'static + Clone,
    {
        let (shared, handler) = Self::new(provider, db, pin_block);
        // spawn the provider handler to a task
        trace!(target: "backendhandler", "spawning Backendhandler task");
        tokio::spawn(handler);
        shared
    }

    /// Same as `Self::spawn_backend` but spawns the `BackendHandler` on a separate `std::thread` in
    /// its own `tokio::Runtime`
    pub fn spawn_backend_thread<T, P>(
        provider: P,
        db: BlockchainDb,
        pin_block: Option<BlockId>,
    ) -> Self
    where
        T: Transport + Clone + Unpin,
        P: Provider<T, AnyNetwork> + Unpin + 'static + Clone,
    {
        let (shared, handler) = Self::new(provider, db, pin_block);

        // spawn a light-weight thread with a thread-local async runtime just for
        // sending and receiving data from the remote client
        std::thread::Builder::new()
            .name("fork-backend".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build tokio runtime");

                rt.block_on(handler);
            })
            .expect("failed to spawn thread");
        trace!(target: "backendhandler", "spawned Backendhandler thread");

        shared
    }

    /// Returns a new `SharedBackend` and the `BackendHandler`
    pub fn new<T, P>(
        provider: P,
        db: BlockchainDb,
        pin_block: Option<BlockId>,
    ) -> (Self, BackendHandler<T, P>)
    where
        T: Transport + Clone + Unpin,
        P: Provider<T, AnyNetwork> + Unpin + 'static + Clone,
    {
        let (backend, backend_rx) = channel(1);
        let cache = Arc::new(FlushJsonBlockCacheDB(Arc::clone(db.cache())));
        let handler = BackendHandler::new(provider, db, backend_rx, pin_block);
        (Self { backend, cache }, handler)
    }

    /// Updates the pinned block to fetch data from
    pub fn set_pinned_block(&self, block: impl Into<BlockId>) -> eyre::Result<()> {
        let req = BackendRequest::SetPinnedBlock(block.into());
        self.backend.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))
    }

    /// Returns the full block for the given block identifier
    pub fn get_full_block(&self, block: impl Into<BlockId>) -> DatabaseResult<Block> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::FullBlock(block.into(), sender);
            self.backend.clone().try_send(req)?;
            rx.recv()?
        })
    }

    /// Returns the transaction for the hash
    pub fn get_transaction(&self, tx: B256) -> DatabaseResult<WithOtherFields<Transaction>> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Transaction(tx, sender);
            self.backend.clone().try_send(req)?;
            rx.recv()?
        })
    }

    fn do_get_basic(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Basic(address, sender);
            self.backend.clone().try_send(req)?;
            rx.recv()?.map(Some)
        })
    }

    fn do_get_storage(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Storage(address, index, sender);
            self.backend.clone().try_send(req)?;
            rx.recv()?
        })
    }

    fn do_get_block_hash(&self, number: u64) -> DatabaseResult<B256> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::BlockHash(number, sender);
            self.backend.clone().try_send(req)?;
            rx.recv()?
        })
    }

    /// Flushes the DB to disk if caching is enabled
    pub(crate) fn flush_cache(&self) {
        self.cache.0.flush();
    }
}

impl DatabaseRef for SharedBackend {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        trace!(target: "sharedbackend", %address, "request basic");
        self.do_get_basic(address).map_err(|err| {
            error!(target: "sharedbackend", %err, %address, "Failed to send/recv `basic`");
            if err.is_possibly_non_archive_node_error() {
                error!(target: "sharedbackend", "{NON_ARCHIVE_NODE_WARNING}");
            }
            err
        })
    }

    fn code_by_hash_ref(&self, hash: B256) -> Result<Bytecode, Self::Error> {
        Err(DatabaseError::MissingCode(hash))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        trace!(target: "sharedbackend", "request storage {:?} at {:?}", address, index);
        self.do_get_storage(address, index).map_err(|err| {
            error!(target: "sharedbackend", %err, %address, %index, "Failed to send/recv `storage`");
            if err.is_possibly_non_archive_node_error() {
                error!(target: "sharedbackend", "{NON_ARCHIVE_NODE_WARNING}");
            }
          err
        })
    }

    fn block_hash_ref(&self, number: U256) -> Result<B256, Self::Error> {
        if number > U256::from(u64::MAX) {
            return Ok(KECCAK_EMPTY);
        }
        let number: U256 = number;
        let number = number.to();
        trace!(target: "sharedbackend", "request block hash for number {:?}", number);
        self.do_get_block_hash(number).map_err(|err| {
            error!(target: "sharedbackend", %err, %number, "Failed to send/recv `block_hash`");
            if err.is_possibly_non_archive_node_error() {
                error!(target: "sharedbackend", "{NON_ARCHIVE_NODE_WARNING}");
            }
            err
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::Backend,
        fork::{BlockchainDbMeta, CreateFork, JsonBlockCacheDB},
        opts::EvmOpts,
    };
    use foundry_common::provider::get_http_provider;
    use foundry_config::{Config, NamedChain};
    use std::{collections::BTreeSet, path::PathBuf};

    const ENDPOINT: Option<&str> = option_env!("ETH_RPC_URL");

    #[tokio::test(flavor = "multi_thread")]
    async fn shared_backend() {
        let Some(endpoint) = ENDPOINT else { return };

        let provider = get_http_provider(endpoint);
        let meta = BlockchainDbMeta {
            cfg_env: Default::default(),
            block_env: Default::default(),
            hosts: BTreeSet::from([endpoint.to_string()]),
        };

        let db = BlockchainDb::new(meta, None);
        let backend = SharedBackend::spawn_backend(Arc::new(provider), db.clone(), None).await;

        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let idx = U256::from(0u64);
        let value = backend.storage_ref(address, idx).unwrap();
        let account = backend.basic_ref(address).unwrap().unwrap();

        let mem_acc = db.accounts().read().get(&address).unwrap().clone();
        assert_eq!(account.balance, mem_acc.balance);
        assert_eq!(account.nonce, mem_acc.nonce);
        let slots = db.storage().read().get(&address).unwrap().clone();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots.get(&idx).copied().unwrap(), value);

        let num = U256::from(10u64);
        let hash = backend.block_hash_ref(num).unwrap();
        let mem_hash = *db.block_hashes().read().get(&num).unwrap();
        assert_eq!(hash, mem_hash);

        let max_slots = 5;
        let handle = std::thread::spawn(move || {
            for i in 1..max_slots {
                let idx = U256::from(i);
                let _ = backend.storage_ref(address, idx);
            }
        });
        handle.join().unwrap();
        let slots = db.storage().read().get(&address).unwrap().clone();
        assert_eq!(slots.len() as u64, max_slots);
    }

    #[test]
    fn can_read_cache() {
        let cache_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/storage.json");
        let json = JsonBlockCacheDB::load(cache_path).unwrap();
        assert!(!json.db().accounts.read().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_read_write_cache() {
        let Some(endpoint) = ENDPOINT else { return };

        let provider = get_http_provider(endpoint);

        let block_num = provider.get_block_number().await.unwrap();

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_block_number = Some(block_num);

        let (env, _block) = evm_opts.fork_evm_env(endpoint).await.unwrap();

        let fork = CreateFork {
            enable_caching: true,
            url: endpoint.to_string(),
            env: env.clone(),
            evm_opts,
        };

        let backend = Backend::spawn(Some(fork));

        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let idx = U256::from(0u64);
        let _value = backend.storage_ref(address, idx);
        let _account = backend.basic_ref(address);

        // fill some slots
        let num_slots = 10u64;
        for idx in 1..num_slots {
            let _ = backend.storage_ref(address, U256::from(idx));
        }
        drop(backend);

        let meta =
            BlockchainDbMeta { cfg_env: env.cfg, block_env: env.block, hosts: Default::default() };

        let db = BlockchainDb::new(
            meta,
            Some(Config::foundry_block_cache_dir(NamedChain::Mainnet, block_num).unwrap()),
        );
        assert!(db.accounts().read().contains_key(&address));
        assert!(db.storage().read().contains_key(&address));
        assert_eq!(db.storage().read().get(&address).unwrap().len(), num_slots as usize);
    }
}
