//! Smart caching and deduplication of requests when using a forking provider
use revm::{db::Database, AccountInfo, KECCAK_EMPTY};

use ethers::{
    core::abi::ethereum_types::BigEndianHash,
    providers::Middleware,
    types::{Address, BlockId, Bytes, H160, H256, U256},
    utils::keccak256,
};
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::{Fuse, Stream, StreamExt},
    task::{Context, Poll},
    Future, FutureExt,
};
use parking_lot::RwLock;
use std::{
    collections::{hash_map::Entry, BTreeMap, HashMap},
    pin::Pin,
    sync::{
        mpsc::{channel as oneshot_channel, Sender as OneshotSender},
        Arc,
    },
};

use foundry_utils::RuntimeOrHandle;

/// A basic in memory cache (address -> AccountInfo + Storage)
pub type MemCache = BTreeMap<H160, (AccountInfo, BTreeMap<U256, U256>)>;

/// A state cache that can be shared across threads
///
/// This can can be used as global state cache.
pub type SharedCache<T> = Arc<RwLock<T>>;

/// Create a new shareable state cache.
///
/// # Example
///
/// ```rust
/// use evm_adapters::sputnik::{MemCache,new_shared_cache};
/// let cache = new_shared_cache(MemCache::default());
/// ```
pub fn new_shared_cache<T>(cache: T) -> SharedCache<T> {
    Arc::new(RwLock::new(cache))
}

type AccountFuture<Err> =
    Pin<Box<dyn Future<Output = (Result<(U256, U256, Bytes), Err>, Address)> + Send>>;
type StorageFuture<Err> = Pin<Box<dyn Future<Output = (Result<U256, Err>, Address, U256)> + Send>>;

/// Request variants that are executed by the provider
enum ProviderRequest<Err> {
    Account(AccountFuture<Err>),
    Storage(StorageFuture<Err>),
}

/// The Request type the Backend listens for
#[derive(Debug)]
enum BackendRequest {
    Basic(Address, OneshotSender<AccountInfo>),
    Storage(Address, U256, OneshotSender<U256>),
}

/// Various types of senders waiting for an answer related to get_account request
enum AccountListener {
    Basic(OneshotSender<AccountInfo>),
}

/// Handles an internal provider and listens for requests.
///
/// This handler will remain active as long as it is reachable (request channel still open) and
/// requests are in progress.
#[must_use = "BackendHandler does nothing unless polled."]
struct BackendHandler<M: Middleware> {
    provider: M,
    /// Stores the state.
    cache: SharedCache<MemCache>,
    /// Requests currently in progress
    pending_requests: Vec<ProviderRequest<M::Error>>,
    /// Listeners that wait for a `get_account` related response
    /// We also store the `get_storage_at` responses until the initial account info is fetched.
    /// The reason for that is because of the simple `address -> Account` model of the cache, so we
    /// only create a new entry for an address of basic info (balance, nonce, code) was fetched.
    account_requests: HashMap<Address, (Vec<AccountListener>, BTreeMap<U256, U256>)>,
    /// Listeners that wait for a `get_storage_at` response
    storage_requests: HashMap<(Address, U256), Vec<OneshotSender<U256>>>,
    /// Incoming commands.
    incoming: Fuse<Receiver<BackendRequest>>,
    /// The block to fetch data from.
    // This is an `Option` so that we can have less code churn in the functions below
    block_id: Option<BlockId>,
}

impl<M> BackendHandler<M>
where
    M: Middleware + Clone + 'static,
{
    fn new(
        provider: M,
        cache: SharedCache<MemCache>,
        rx: Receiver<BackendRequest>,
        block_id: Option<BlockId>,
    ) -> Self {
        Self {
            provider,
            cache,
            pending_requests: Default::default(),
            account_requests: Default::default(),
            storage_requests: Default::default(),
            incoming: rx.fuse(),
            block_id,
        }
    }

    /// handle the request in queue in the future.
    ///
    /// We always check:
    ///  1. if the requested value is already stored in the cache, then answer the sender
    ///  2. otherwise, fetch it via the provider but check if a request for that value is already in
    /// progress (e.g. another Sender just requested the same account)
    fn on_request(&mut self, req: BackendRequest) {
        match req {
            BackendRequest::Basic(addr, sender) => {
                let lock = self.cache.read();
                let basic = lock.get(&addr).cloned();
                // release the lock
                drop(lock);
                if let Some(basic) = basic {
                    let _ = sender.send(basic.0);
                } else {
                    self.request_account(addr, AccountListener::Basic(sender));
                }
            }
            BackendRequest::Storage(addr, idx, sender) => {
                let lock = self.cache.read();
                let acc = lock.get(&addr);
                let has_account = acc.is_some();
                let value = acc.and_then(|acc| acc.1.get(&idx).copied());
                // release the lock
                drop(lock);

                if has_account {
                    // account is already stored in the cache
                    if let Some(value) = value {
                        let _ = sender.send(value);
                    } else {
                        // account present but not storage -> fetch storage
                        self.request_account_storage(addr, idx, sender);
                    }
                } else {
                    // account is still missing in the cache
                    // check if already fetched but not in cache yet
                    if let Some(value) =
                        self.account_requests.get(&addr).and_then(|(_, s)| s.get(&idx).copied())
                    {
                        let _ = sender.send(value);
                    } else {
                        // fetch storage via provider
                        self.request_account_storage(addr, idx, sender);
                    }
                }
            }
        }
    }

    /// process a request for account's storage
    fn request_account_storage(
        &mut self,
        address: Address,
        idx: U256,
        listener: OneshotSender<U256>,
    ) {
        match self.storage_requests.entry((address, idx)) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(listener);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![listener]);
                let provider = self.provider.clone();
                let block_id = self.block_id;
                let fut = Box::pin(async move {
                    // serialize & deserialize back to U256
                    let idx_req = H256::from_uint(&idx);
                    let storage = provider.get_storage_at(address, idx_req, block_id).await;
                    let storage = storage.map(|storage| storage.into_uint());
                    (storage, address, idx)
                });
                self.pending_requests.push(ProviderRequest::Storage(fut));
            }
        }
    }

    /// returns the future that fetches the account data
    fn get_account_req(&self, address: Address) -> ProviderRequest<M::Error> {
        let provider = self.provider.clone();
        let block_id = self.block_id;
        let fut = Box::pin(async move {
            let balance = provider.get_balance(address, block_id);
            let nonce = provider.get_transaction_count(address, block_id);
            let code = provider.get_code(address, block_id);
            let resp = tokio::try_join!(balance, nonce, code);
            (resp, address)
        });
        ProviderRequest::Account(fut)
    }

    /// process a request for an account
    fn request_account(&mut self, address: Address, listener: AccountListener) {
        match self.account_requests.entry(address) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().0.push(listener);
            }
            Entry::Vacant(entry) => {
                entry.insert((vec![listener], Default::default()));
                self.pending_requests.push(self.get_account_req(address));
            }
        }
    }
}

impl<M> Future for BackendHandler<M>
where
    M: Middleware + Clone + Unpin + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // receive new requests to delegate to the underlying provider
        while let Poll::Ready(Some(req)) = Pin::new(&mut pin.incoming).poll_next(cx) {
            pin.on_request(req)
        }

        // poll all requests in progress
        for n in (0..pin.pending_requests.len()).rev() {
            let mut request = pin.pending_requests.swap_remove(n);
            match &mut request {
                ProviderRequest::Account(fut) => {
                    if let Poll::Ready((resp, addr)) = fut.poll_unpin(cx) {
                        // get the response
                        let (balance, nonce, code) = resp.unwrap_or_else(|_| {
                            tracing::trace!("Failed to get account for {}", addr);
                            Default::default()
                        });

                        // conver it to revm-style types
                        let (code, code_hash) = if !code.0.is_empty() {
                            (Some(code.0.clone()), keccak256(&code).into())
                        } else {
                            (None, KECCAK_EMPTY)
                        };

                        let (listeners, storage) =
                            pin.account_requests.remove(&addr).unwrap_or_default();
                        let acc = AccountInfo { nonce: nonce.as_u64(), balance, code, code_hash };
                        pin.cache.write().insert(addr, (acc.clone(), storage));
                        // notify all listeners
                        for listener in listeners {
                            match listener {
                                AccountListener::Basic(sender) => {
                                    let _ = sender.send(acc.clone());
                                }
                            }
                        }
                        continue
                    }
                }
                ProviderRequest::Storage(fut) => {
                    if let Poll::Ready((resp, addr, idx)) = fut.poll_unpin(cx) {
                        let value = resp.unwrap_or_else(|_| {
                            tracing::trace!("Failed to get storage for {} at {}", addr, idx);
                            Default::default()
                        });
                        if let Some(acc) = pin.cache.write().get_mut(&addr) {
                            acc.1.insert(idx, value);
                        } else {
                            // the account not fetched yet, we either add this value to the storage
                            // buffer of the request in progress or start the `get_account` request
                            match pin.account_requests.entry(addr) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().1.insert(idx, value);
                                }
                                Entry::Vacant(entry) => {
                                    let mut storage = BTreeMap::new();
                                    storage.insert(idx, value);
                                    entry.insert((vec![], storage));
                                    pin.pending_requests.push(pin.get_account_req(addr));
                                }
                            }
                        }
                        // notify all listeners
                        if let Some(listeners) = pin.storage_requests.remove(&(addr, idx)) {
                            listeners.into_iter().for_each(|l| {
                                let _ = l.send(value);
                            })
                        }
                        continue
                    }
                }
            }
            // not ready, insert and poll again
            pin.pending_requests.push(request);
        }
        // the handler is finished if the request channel was closed and all requests are processed
        if pin.incoming.is_done() && pin.pending_requests.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// A cloneable backend type that shares access to the backend data with all its clones.
///
/// This backend type is connected to the `BackendHandler` via a mpsc channel. The `BackendHandlers`
/// is spawned on a background thread and listens for incoming commands on the receiver half of the
/// channel. A `SharedBackend` holds a sender for that channel, which is `Clone`, so their can be
/// multiple `SharedBackend`s communicating with the same `BackendHandler`, hence this `Backend`
/// type is thread safe.
///
/// All `Backend` trait functions are delegated as a `BackendRequest` via the channel to the
/// `BackendHandler`. All `BackendRequest` variants include a sender half of an additional channel
/// that is used by the `BackendHandler` to send the result of an executed `BackendRequest` back to
/// `SharedBackend`.
///
/// The `BackendHandler` holds an ethers `Provider` to look up missing accounts or storage slots
/// from remote (e.g. infura). It detects duplicate requests from multiple `SharedBackend`s and
/// bundles them together, so that always only one provider request is executed. For example, there
/// are two `SharedBackend`s, `A` and `B`, both request the basic account info of account
/// `0xasd9sa7d...` at the same time. After the `BackendHandler` receives the request from `A`, it
/// sends a new provider request to the provider's endpoint, then it reads the identical request
/// from `B` and simply adds it as an additional listener for the request already in progress,
/// instead of sending another one. So that after the provider returns the response all listeners
/// (`A` and `B`) get notified.
#[derive(Debug, Clone)]
pub struct SharedBackend<DB> {
    inner: SharedBackendInner<DB>,
}

impl<DB: Database> SharedBackend<DB> {
    /// Spawns a new `BackendHandler` on a background thread that listens for requests from any
    /// `SharedBackend`. Missing values get inserted in the `cache`.
    ///
    /// NOTE: this should be called with `Arc<Provider>`
    pub fn new<M>(
        provider: M,
        cache: SharedCache<MemCache>,
        db: DB,
        pin_block: Option<BlockId>,
    ) -> Self
    where
        M: Middleware + Unpin + 'static + Clone,
    {
        let (tx, rx) = channel(1);
        let handler = BackendHandler::new(provider, cache, rx, pin_block);
        // spawn the provider handler to background
        let rt = RuntimeOrHandle::new();
        std::thread::spawn(move || match rt {
            RuntimeOrHandle::Runtime(runtime) => runtime.block_on(handler),
            RuntimeOrHandle::Handle(handle) => handle.block_on(handler),
        });

        Self { inner: SharedBackendInner { db, backend: tx } }
    }

    fn do_get_basic(&self, address: H160) -> eyre::Result<AccountInfo> {
        let (sender, rx) = oneshot_channel();
        let req = BackendRequest::Basic(address, sender);
        self.inner.backend.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }

    fn do_get_storage(&self, address: H160, index: U256) -> eyre::Result<U256> {
        let (sender, rx) = oneshot_channel();
        let req = BackendRequest::Storage(address, index, sender);
        self.inner.backend.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }
}

impl<DB: Database> Database for SharedBackend<DB> {
    fn block_hash(&mut self, number: U256) -> H256 {
        self.inner.db.block_hash(number)
    }

    fn basic(&mut self, address: H160) -> AccountInfo {
        self.do_get_basic(address).unwrap_or_else(|_| {
            tracing::trace!("Failed to send/recv `basic` for {}", address);
            Default::default()
        })
    }

    fn code_by_hash(&mut self, _address: H256) -> bytes::Bytes {
        panic!("Should not be called. Code is already loaded.")
    }

    fn storage(&mut self, address: H160, index: U256) -> U256 {
        self.do_get_storage(address, index).unwrap_or_else(|_| {
            tracing::trace!("Failed to send/recv `storage` for {} at {}", address, index);
            Default::default()
        })
    }
}

#[derive(Debug, Clone)]
struct SharedBackendInner<DB> {
    db: DB,
    backend: Sender<BackendRequest>,
}

#[cfg(test)]
mod tests {
    use ethers::{
        providers::{Http, Provider},
        types::Address,
    };
    use std::convert::TryFrom;

    use super::*;

    #[test]
    fn shared_backend() {
        let provider = Provider::<Http>::try_from(
            "https://mainnet.infura.io/v3/c60b0bb42f8a4c6481ecd229eddaca27",
        )
        .unwrap();
        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let cache = new_shared_cache(MemCache::default());
        let db = revm::InMemoryDB::default();

        let mut backend = SharedBackend::new(Arc::new(provider), cache.clone(), db, None);

        let idx = U256::from(0u64);
        let value = backend.storage(address, idx);
        let account = backend.basic(address);

        let mem_acc = cache.read().get(&address).unwrap().clone();
        assert_eq!(account.balance, mem_acc.0.balance);
        assert_eq!(account.nonce, mem_acc.0.nonce);
        assert_eq!(mem_acc.1.len(), 1);
        assert_eq!(mem_acc.1.get(&idx).copied().unwrap(), value);

        let mut backend = backend.clone();
        let max_slots = 5;
        let handle = std::thread::spawn(move || {
            for i in 1..max_slots {
                let idx = U256::from(i);
                let _ = backend.storage(address, idx);
            }
        });
        handle.join().unwrap();
        let mem_acc = cache.read().get(&address).unwrap().clone();
        assert_eq!(mem_acc.1.len() as u64, max_slots);
    }
}
