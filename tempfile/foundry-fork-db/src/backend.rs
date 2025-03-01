//! Smart caching and deduplication of requests when using a forking provider.

use crate::{
    cache::{BlockchainDb, FlushJsonBlockCacheDB, MemDb, StorageInfo},
    error::{DatabaseError, DatabaseResult},
};
use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_provider::{
    network::{AnyNetwork, AnyRpcBlock, AnyRpcTransaction, AnyTxEnvelope},
    Provider,
};
use alloy_rpc_types::{BlockId, Transaction};
use alloy_serde::WithOtherFields;
use eyre::WrapErr;
use futures::{
    channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    stream::Stream,
    task::{Context, Poll},
    Future, FutureExt,
};
use revm::{
    db::DatabaseRef,
    primitives::{
        map::{hash_map::Entry, AddressHashMap, HashMap},
        AccountInfo, Bytecode, KECCAK_EMPTY,
    },
};
use std::{
    collections::VecDeque,
    fmt,
    future::IntoFuture,
    path::Path,
    pin::Pin,
    sync::{
        mpsc::{channel as oneshot_channel, Sender as OneshotSender},
        Arc,
    },
};

/// Logged when an error is indicative that the user is trying to fork from a non-archive node.
pub const NON_ARCHIVE_NODE_WARNING: &str = "\
It looks like you're trying to fork from an older block with a non-archive node which is not \
supported. Please try to change your RPC url to an archive node if the issue persists.";

// Various future/request type aliases

type AccountFuture<Err> =
    Pin<Box<dyn Future<Output = (Result<(U256, u64, Bytes), Err>, Address)> + Send>>;
type StorageFuture<Err> = Pin<Box<dyn Future<Output = (Result<U256, Err>, Address, U256)> + Send>>;
type BlockHashFuture<Err> = Pin<Box<dyn Future<Output = (Result<B256, Err>, u64)> + Send>>;
type FullBlockFuture<Err> = Pin<
    Box<dyn Future<Output = (FullBlockSender, Result<Option<AnyRpcBlock>, Err>, BlockId)> + Send>,
>;
type TransactionFuture<Err> =
    Pin<Box<dyn Future<Output = (TransactionSender, Result<AnyRpcTransaction, Err>, B256)> + Send>>;

type AccountInfoSender = OneshotSender<DatabaseResult<AccountInfo>>;
type StorageSender = OneshotSender<DatabaseResult<U256>>;
type BlockHashSender = OneshotSender<DatabaseResult<B256>>;
type FullBlockSender = OneshotSender<DatabaseResult<AnyRpcBlock>>;
type TransactionSender = OneshotSender<DatabaseResult<AnyRpcTransaction>>;

type AddressData = AddressHashMap<AccountInfo>;
type StorageData = AddressHashMap<StorageInfo>;
type BlockHashData = HashMap<U256, B256>;

struct AnyRequestFuture<T, Err> {
    sender: OneshotSender<Result<T, Err>>,
    future: Pin<Box<dyn Future<Output = Result<T, Err>> + Send>>,
}

impl<T, Err> fmt::Debug for AnyRequestFuture<T, Err> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AnyRequestFuture").field(&self.sender).finish()
    }
}

trait WrappedAnyRequest: Unpin + Send + fmt::Debug {
    fn poll_inner(&mut self, cx: &mut Context<'_>) -> Poll<()>;
}

/// @dev Implements `WrappedAnyRequest` for `AnyRequestFuture`.
///
/// - `poll_inner` is similar to `Future` polling but intentionally consumes the Future<Output=T>
///   and return Future<Output=()>
/// - This design avoids storing `Future<Output = T>` directly, as its type may not be known at
///   compile time.
/// - Instead, the result (`Result<T, Err>`) is sent via the `sender` channel, which enforces type
///   safety.
impl<T, Err> WrappedAnyRequest for AnyRequestFuture<T, Err>
where
    T: fmt::Debug + Send + 'static,
    Err: fmt::Debug + Send + 'static,
{
    fn poll_inner(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        match self.future.poll_unpin(cx) {
            Poll::Ready(result) => {
                let _ = self.sender.send(result);
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Request variants that are executed by the provider
enum ProviderRequest<Err> {
    Account(AccountFuture<Err>),
    Storage(StorageFuture<Err>),
    BlockHash(BlockHashFuture<Err>),
    FullBlock(FullBlockFuture<Err>),
    Transaction(TransactionFuture<Err>),
    AnyRequest(Box<dyn WrappedAnyRequest>),
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

    /// Update Address data
    UpdateAddress(AddressData),
    /// Update Storage data
    UpdateStorage(StorageData),
    /// Update Block Hashes
    UpdateBlockHash(BlockHashData),
    /// Any other request
    AnyRequest(Box<dyn WrappedAnyRequest>),
}

/// Handles an internal provider and listens for requests.
///
/// This handler will remain active as long as it is reachable (request channel still open) and
/// requests are in progress.
#[must_use = "futures do nothing unless polled"]
pub struct BackendHandler<P> {
    provider: P,
    /// Stores all the data.
    db: BlockchainDb,
    /// Requests currently in progress
    pending_requests: Vec<ProviderRequest<eyre::Report>>,
    /// Listeners that wait for a `get_account` related response
    account_requests: HashMap<Address, Vec<AccountInfoSender>>,
    /// Listeners that wait for a `get_storage_at` response
    storage_requests: HashMap<(Address, U256), Vec<StorageSender>>,
    /// Listeners that wait for a `get_block` response
    block_requests: HashMap<u64, Vec<BlockHashSender>>,
    /// Incoming commands.
    incoming: UnboundedReceiver<BackendRequest>,
    /// unprocessed queued requests
    queued_requests: VecDeque<BackendRequest>,
    /// The block to fetch data from.
    // This is an `Option` so that we can have less code churn in the functions below
    block_id: Option<BlockId>,
}

impl<P> BackendHandler<P>
where
    P: Provider<AnyNetwork> + Clone + Unpin + 'static,
{
    fn new(
        provider: P,
        db: BlockchainDb,
        rx: UnboundedReceiver<BackendRequest>,
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
            BackendRequest::UpdateAddress(address_data) => {
                for (address, data) in address_data {
                    self.db.accounts().write().insert(address, data);
                }
            }
            BackendRequest::UpdateStorage(storage_data) => {
                for (address, data) in storage_data {
                    self.db.storage().write().insert(address, data);
                }
            }
            BackendRequest::UpdateBlockHash(block_hash_data) => {
                for (block, hash) in block_hash_data {
                    self.db.block_hashes().write().insert(block, hash);
                }
            }
            BackendRequest::AnyRequest(fut) => {
                self.pending_requests.push(ProviderRequest::AnyRequest(fut));
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
                .wrap_err(format!("could not fetch block {number:?}"));
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
                        .get_block_by_number(
                            number.into(),
                            alloy_rpc_types::BlockTransactionsKind::Hashes,
                        )
                        .await
                        .wrap_err("failed to get block");

                    let block_hash = match block {
                        Ok(Some(block)) => Ok(block.header.hash),
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

impl<P> Future for BackendHandler<P>
where
    P: Provider<AnyNetwork> + Clone + Unpin + 'static,
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
                    ProviderRequest::AnyRequest(fut) => {
                        if fut.poll_inner(cx).is_ready() {
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

/// Mode for the `SharedBackend` how to block in the non-async [`DatabaseRef`] when interacting with
/// [`BackendHandler`].
#[derive(Default, Clone, Debug, PartialEq)]
pub enum BlockingMode {
    /// This mode use `tokio::task::block_in_place()` to block in place.
    ///
    /// This should be used when blocking on the call site is disallowed.
    #[default]
    BlockInPlace,
    /// The mode blocks the current task
    ///
    /// This can be used if blocking on the call site is allowed, e.g. on a tokio blocking task.
    Block,
}

impl BlockingMode {
    /// run process logic with the blocking mode
    pub fn run<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        match self {
            Self::BlockInPlace => tokio::task::block_in_place(f),
            Self::Block => f(),
        }
    }
}

/// A cloneable backend type that shares access to the backend data with all its clones.
///
/// This backend type is connected to the `BackendHandler` via a mpsc unbounded channel. The
/// `BackendHandler` is spawned on a tokio task and listens for incoming commands on the receiver
/// half of the channel. A `SharedBackend` holds a sender for that channel, which is `Clone`, so
/// there can be multiple `SharedBackend`s communicating with the same `BackendHandler`, hence this
/// `Backend` type is thread safe.
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
    backend: UnboundedSender<BackendRequest>,
    /// Ensures that the underlying cache gets flushed once the last `SharedBackend` is dropped.
    ///
    /// There is only one instance of the type, so as soon as the last `SharedBackend` is deleted,
    /// `FlushJsonBlockCacheDB` is also deleted and the cache is flushed.
    cache: Arc<FlushJsonBlockCacheDB>,

    /// The mode for the `SharedBackend` to block in place or not
    blocking_mode: BlockingMode,
}

impl SharedBackend {
    /// _Spawns_ a new `BackendHandler` on a `tokio::task` that listens for requests from any
    /// `SharedBackend`. Missing values get inserted in the `db`.
    ///
    /// The spawned `BackendHandler` finishes once the last `SharedBackend` connected to it is
    /// dropped.
    ///
    /// NOTE: this should be called with `Arc<Provider>`
    pub async fn spawn_backend<P>(provider: P, db: BlockchainDb, pin_block: Option<BlockId>) -> Self
    where
        P: Provider<AnyNetwork> + Unpin + 'static + Clone,
    {
        let (shared, handler) = Self::new(provider, db, pin_block);
        // spawn the provider handler to a task
        trace!(target: "backendhandler", "spawning Backendhandler task");
        tokio::spawn(handler);
        shared
    }

    /// Same as `Self::spawn_backend` but spawns the `BackendHandler` on a separate `std::thread` in
    /// its own `tokio::Runtime`
    pub fn spawn_backend_thread<P>(
        provider: P,
        db: BlockchainDb,
        pin_block: Option<BlockId>,
    ) -> Self
    where
        P: Provider<AnyNetwork> + Unpin + 'static + Clone,
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
    pub fn new<P>(
        provider: P,
        db: BlockchainDb,
        pin_block: Option<BlockId>,
    ) -> (Self, BackendHandler<P>)
    where
        P: Provider<AnyNetwork> + Unpin + 'static + Clone,
    {
        let (backend, backend_rx) = unbounded();
        let cache = Arc::new(FlushJsonBlockCacheDB(Arc::clone(db.cache())));
        let handler = BackendHandler::new(provider, db, backend_rx, pin_block);
        (Self { backend, cache, blocking_mode: Default::default() }, handler)
    }

    /// Returns a new `SharedBackend` and the `BackendHandler` with a specific blocking mode
    pub fn with_blocking_mode(&self, mode: BlockingMode) -> Self {
        Self { backend: self.backend.clone(), cache: self.cache.clone(), blocking_mode: mode }
    }

    /// Updates the pinned block to fetch data from
    pub fn set_pinned_block(&self, block: impl Into<BlockId>) -> eyre::Result<()> {
        let req = BackendRequest::SetPinnedBlock(block.into());
        self.backend.unbounded_send(req).map_err(|e| eyre::eyre!("{:?}", e))
    }

    /// Returns the full block for the given block identifier
    pub fn get_full_block(&self, block: impl Into<BlockId>) -> DatabaseResult<AnyRpcBlock> {
        self.blocking_mode.run(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::FullBlock(block.into(), sender);
            self.backend.unbounded_send(req)?;
            rx.recv()?
        })
    }

    /// Returns the transaction for the hash
    pub fn get_transaction(
        &self,
        tx: B256,
    ) -> DatabaseResult<WithOtherFields<Transaction<AnyTxEnvelope>>> {
        self.blocking_mode.run(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Transaction(tx, sender);
            self.backend.unbounded_send(req)?;
            rx.recv()?
        })
    }

    fn do_get_basic(&self, address: Address) -> DatabaseResult<Option<AccountInfo>> {
        self.blocking_mode.run(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Basic(address, sender);
            self.backend.unbounded_send(req)?;
            rx.recv()?.map(Some)
        })
    }

    fn do_get_storage(&self, address: Address, index: U256) -> DatabaseResult<U256> {
        self.blocking_mode.run(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Storage(address, index, sender);
            self.backend.unbounded_send(req)?;
            rx.recv()?
        })
    }

    fn do_get_block_hash(&self, number: u64) -> DatabaseResult<B256> {
        self.blocking_mode.run(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::BlockHash(number, sender);
            self.backend.unbounded_send(req)?;
            rx.recv()?
        })
    }

    /// Inserts or updates data for multiple addresses
    pub fn insert_or_update_address(&self, address_data: AddressData) {
        let req = BackendRequest::UpdateAddress(address_data);
        let err = self.backend.unbounded_send(req);
        match err {
            Ok(_) => (),
            Err(e) => {
                error!(target: "sharedbackend", "Failed to send update address request: {:?}", e)
            }
        }
    }

    /// Inserts or updates data for multiple storage slots
    pub fn insert_or_update_storage(&self, storage_data: StorageData) {
        let req = BackendRequest::UpdateStorage(storage_data);
        let err = self.backend.unbounded_send(req);
        match err {
            Ok(_) => (),
            Err(e) => {
                error!(target: "sharedbackend", "Failed to send update address request: {:?}", e)
            }
        }
    }

    /// Inserts or updates data for multiple block hashes
    pub fn insert_or_update_block_hashes(&self, block_hash_data: BlockHashData) {
        let req = BackendRequest::UpdateBlockHash(block_hash_data);
        let err = self.backend.unbounded_send(req);
        match err {
            Ok(_) => (),
            Err(e) => {
                error!(target: "sharedbackend", "Failed to send update address request: {:?}", e)
            }
        }
    }

    /// Returns any arbitrary request on the provider
    pub fn do_any_request<T, F>(&mut self, fut: F) -> DatabaseResult<T>
    where
        F: Future<Output = Result<T, eyre::Report>> + Send + 'static,
        T: fmt::Debug + Send + 'static,
    {
        self.blocking_mode.run(|| {
            let (sender, rx) = oneshot_channel::<Result<T, eyre::Report>>();
            let req = BackendRequest::AnyRequest(Box::new(AnyRequestFuture {
                sender,
                future: Box::pin(fut),
            }));
            self.backend.unbounded_send(req)?;
            rx.recv()?.map_err(|err| DatabaseError::AnyRequest(Arc::new(err)))
        })
    }

    /// Flushes the DB to disk if caching is enabled
    pub fn flush_cache(&self) {
        self.cache.0.flush();
    }

    /// Flushes the DB to a specific file
    pub fn flush_cache_to(&self, cache_path: &Path) {
        self.cache.0.flush_to(cache_path);
    }

    /// Returns the DB
    pub fn data(&self) -> Arc<MemDb> {
        self.cache.0.db().clone()
    }

    /// Returns the DB accounts
    pub fn accounts(&self) -> AddressData {
        self.cache.0.db().accounts.read().clone()
    }

    /// Returns the DB accounts length
    pub fn accounts_len(&self) -> usize {
        self.cache.0.db().accounts.read().len()
    }

    /// Returns the DB storage
    pub fn storage(&self) -> StorageData {
        self.cache.0.db().storage.read().clone()
    }

    /// Returns the DB storage length
    pub fn storage_len(&self) -> usize {
        self.cache.0.db().storage.read().len()
    }

    /// Returns the DB block_hashes
    pub fn block_hashes(&self) -> BlockHashData {
        self.cache.0.db().block_hashes.read().clone()
    }

    /// Returns the DB block_hashes length
    pub fn block_hashes_len(&self) -> usize {
        self.cache.0.db().block_hashes.read().len()
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

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
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
    use crate::cache::{BlockchainDbMeta, JsonBlockCacheDB};
    use alloy_provider::ProviderBuilder;
    use alloy_rpc_client::ClientBuilder;
    use serde::Deserialize;
    use std::{collections::BTreeSet, fs, path::PathBuf};
    use tiny_http::{Response, Server};

    pub fn get_http_provider(endpoint: &str) -> impl Provider<AnyNetwork> + Clone {
        ProviderBuilder::new()
            .network::<AnyNetwork>()
            .on_client(ClientBuilder::default().http(endpoint.parse().unwrap()))
    }

    const ENDPOINT: Option<&str> = option_env!("ETH_RPC_URL");

    #[tokio::test(flavor = "multi_thread")]
    async fn test_builder() {
        let Some(endpoint) = ENDPOINT else { return };
        let provider = get_http_provider(endpoint);

        let any_rpc_block = provider
            .get_block(BlockId::latest(), alloy_rpc_types::BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        let _meta = BlockchainDbMeta::default().with_block(&any_rpc_block.inner);
    }

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

        let num = 10u64;
        let hash = backend.block_hash_ref(num).unwrap();
        let mem_hash = *db.block_hashes().read().get(&U256::from(num)).unwrap();
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
    async fn can_modify_address() {
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

        let new_acc = AccountInfo {
            nonce: 1000u64,
            balance: U256::from(2000),
            code: None,
            code_hash: KECCAK_EMPTY,
        };
        let mut account_data = AddressData::default();
        account_data.insert(address, new_acc.clone());

        backend.insert_or_update_address(account_data);

        let max_slots = 5;
        let handle = std::thread::spawn(move || {
            for i in 1..max_slots {
                let idx = U256::from(i);
                let result_address = backend.basic_ref(address).unwrap();
                match result_address {
                    Some(acc) => {
                        assert_eq!(
                            acc.nonce, new_acc.nonce,
                            "The nonce was not changed in instance of index {}",
                            idx
                        );
                        assert_eq!(
                            acc.balance, new_acc.balance,
                            "The balance was not changed in instance of index {}",
                            idx
                        );

                        // comparing with db
                        let db_address = {
                            let accounts = db.accounts().read();
                            accounts.get(&address).unwrap().clone()
                        };

                        assert_eq!(
                            db_address.nonce, new_acc.nonce,
                            "The nonce was not changed in instance of index {}",
                            idx
                        );
                        assert_eq!(
                            db_address.balance, new_acc.balance,
                            "The balance was not changed in instance of index {}",
                            idx
                        );
                    }
                    None => panic!("Account not found"),
                }
            }
        });
        handle.join().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_modify_storage() {
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

        let mut storage_data = StorageData::default();
        let mut storage_info = StorageInfo::default();
        storage_info.insert(U256::from(20), U256::from(10));
        storage_info.insert(U256::from(30), U256::from(15));
        storage_info.insert(U256::from(40), U256::from(20));

        storage_data.insert(address, storage_info);

        backend.insert_or_update_storage(storage_data.clone());

        let max_slots = 5;
        let handle = std::thread::spawn(move || {
            for _ in 1..max_slots {
                for (address, info) in &storage_data {
                    for (index, value) in info {
                        let result_storage = backend.do_get_storage(*address, *index);
                        match result_storage {
                            Ok(stg_db) => {
                                assert_eq!(
                                    stg_db, *value,
                                    "Storage in slot number {} in address {} do not have the same value", index, address
                                );

                                let db_result = {
                                    let storage = db.storage().read();
                                    let address_storage = storage.get(address).unwrap();
                                    *address_storage.get(index).unwrap()
                                };

                                assert_eq!(
                                    stg_db, db_result,
                                    "Storage in slot number {} in address {} do not have the same value", index, address
                                )
                            }

                            Err(err) => {
                                panic!("There was a database error: {}", err)
                            }
                        }
                    }
                }
            }
        });
        handle.join().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_modify_block_hashes() {
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
        // let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let mut block_hash_data = BlockHashData::default();
        block_hash_data.insert(U256::from(1), B256::from(U256::from(1)));
        block_hash_data.insert(U256::from(2), B256::from(U256::from(2)));
        block_hash_data.insert(U256::from(3), B256::from(U256::from(3)));
        block_hash_data.insert(U256::from(4), B256::from(U256::from(4)));
        block_hash_data.insert(U256::from(5), B256::from(U256::from(5)));

        backend.insert_or_update_block_hashes(block_hash_data.clone());

        let max_slots: u64 = 5;
        let handle = std::thread::spawn(move || {
            for i in 1..max_slots {
                let key = U256::from(i);
                let result_hash = backend.do_get_block_hash(i);
                match result_hash {
                    Ok(hash) => {
                        assert_eq!(
                            hash,
                            *block_hash_data.get(&key).unwrap(),
                            "The hash in block {} did not match",
                            key
                        );

                        let db_result = {
                            let hashes = db.block_hashes().read();
                            *hashes.get(&key).unwrap()
                        };

                        assert_eq!(hash, db_result, "The hash in block {} did not match", key);
                    }
                    Err(err) => panic!("Hash not found, error: {}", err),
                }
            }
        });
        handle.join().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_modify_storage_with_cache() {
        let Some(endpoint) = ENDPOINT else { return };

        let provider = get_http_provider(endpoint);
        let meta = BlockchainDbMeta {
            cfg_env: Default::default(),
            block_env: Default::default(),
            hosts: BTreeSet::from([endpoint.to_string()]),
        };

        // create a temporary file
        fs::copy("test-data/storage.json", "test-data/storage-tmp.json").unwrap();

        let cache_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/storage-tmp.json");

        let db = BlockchainDb::new(meta.clone(), Some(cache_path));
        let backend =
            SharedBackend::spawn_backend(Arc::new(provider.clone()), db.clone(), None).await;

        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let mut storage_data = StorageData::default();
        let mut storage_info = StorageInfo::default();
        storage_info.insert(U256::from(1), U256::from(10));
        storage_info.insert(U256::from(2), U256::from(15));
        storage_info.insert(U256::from(3), U256::from(20));
        storage_info.insert(U256::from(4), U256::from(20));
        storage_info.insert(U256::from(5), U256::from(15));
        storage_info.insert(U256::from(6), U256::from(10));

        let mut address_data = backend.basic_ref(address).unwrap().unwrap();
        address_data.code = None;

        storage_data.insert(address, storage_info);

        backend.insert_or_update_storage(storage_data.clone());

        let mut new_acc = backend.basic_ref(address).unwrap().unwrap();
        // nullify the code
        new_acc.code = Some(Bytecode::new_raw(([10, 20, 30, 40]).into()));

        let mut account_data = AddressData::default();
        account_data.insert(address, new_acc.clone());

        backend.insert_or_update_address(account_data);

        let backend_clone = backend.clone();

        let max_slots = 5;
        let handle = std::thread::spawn(move || {
            for _ in 1..max_slots {
                for (address, info) in &storage_data {
                    for (index, value) in info {
                        let result_storage = backend.do_get_storage(*address, *index);
                        match result_storage {
                            Ok(stg_db) => {
                                assert_eq!(
                                    stg_db, *value,
                                    "Storage in slot number {} in address {} doesn't have the same value", index, address
                                );

                                let db_result = {
                                    let storage = db.storage().read();
                                    let address_storage = storage.get(address).unwrap();
                                    *address_storage.get(index).unwrap()
                                };

                                assert_eq!(
                                    stg_db, db_result,
                                    "Storage in slot number {} in address {} doesn't have the same value", index, address
                                );
                            }

                            Err(err) => {
                                panic!("There was a database error: {}", err)
                            }
                        }
                    }
                }
            }

            backend_clone.flush_cache();
        });
        handle.join().unwrap();

        // read json and confirm the changes to the data

        let cache_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/storage-tmp.json");

        let json_db = BlockchainDb::new(meta, Some(cache_path));

        let mut storage_data = StorageData::default();
        let mut storage_info = StorageInfo::default();
        storage_info.insert(U256::from(1), U256::from(10));
        storage_info.insert(U256::from(2), U256::from(15));
        storage_info.insert(U256::from(3), U256::from(20));
        storage_info.insert(U256::from(4), U256::from(20));
        storage_info.insert(U256::from(5), U256::from(15));
        storage_info.insert(U256::from(6), U256::from(10));

        storage_data.insert(address, storage_info);

        // redo the checks with the data extracted from the json file
        let max_slots = 5;
        let handle = std::thread::spawn(move || {
            for _ in 1..max_slots {
                for (address, info) in &storage_data {
                    for (index, value) in info {
                        let result_storage = {
                            let storage = json_db.storage().read();
                            let address_storage = storage.get(address).unwrap().clone();
                            *address_storage.get(index).unwrap()
                        };

                        assert_eq!(
                            result_storage, *value,
                            "Storage in slot number {} in address {} doesn't have the same value",
                            index, address
                        );
                    }
                }
            }
        });

        handle.join().unwrap();

        // erase the temporary file
        fs::remove_file("test-data/storage-tmp.json").unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shared_backend_any_request() {
        let expected_response_bytes: Bytes = vec![0xff, 0xee].into();
        let server = Server::http("0.0.0.0:0").expect("failed starting in-memory http server");
        let endpoint = format!("http://{}", server.server_addr());

        // Spin an in-memory server that responds to "foo_callCustomMethod" rpc call.
        let expected_bytes_innner = expected_response_bytes.clone();
        let server_handle = std::thread::spawn(move || {
            #[derive(Debug, Deserialize)]
            struct Request {
                method: String,
            }
            let mut request = server.recv().unwrap();
            let rpc_request: Request =
                serde_json::from_reader(request.as_reader()).expect("failed parsing request");

            match rpc_request.method.as_str() {
                "foo_callCustomMethod" => request
                    .respond(Response::from_string(format!(
                        r#"{{"result": "{}"}}"#,
                        alloy_primitives::hex::encode_prefixed(expected_bytes_innner),
                    )))
                    .unwrap(),
                _ => request
                    .respond(Response::from_string(r#"{"error": "invalid request"}"#))
                    .unwrap(),
            };
        });

        let provider = get_http_provider(&endpoint);
        let meta = BlockchainDbMeta {
            cfg_env: Default::default(),
            block_env: Default::default(),
            hosts: BTreeSet::from([endpoint.to_string()]),
        };

        let db = BlockchainDb::new(meta, None);
        let provider_inner = provider.clone();
        let mut backend = SharedBackend::spawn_backend(Arc::new(provider), db.clone(), None).await;

        let actual_response_bytes = backend
            .do_any_request(async move {
                let bytes: alloy_primitives::Bytes =
                    provider_inner.raw_request("foo_callCustomMethod".into(), vec!["0001"]).await?;
                Ok(bytes)
            })
            .expect("failed performing any request");

        assert_eq!(actual_response_bytes, expected_response_bytes);

        server_handle.join().unwrap();
    }
}
