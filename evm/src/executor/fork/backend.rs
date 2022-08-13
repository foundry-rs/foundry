//! Smart caching and deduplication of requests when using a forking provider
use crate::executor::fork::{cache::FlushJsonBlockCacheDB, BlockchainDb};
use ethers::{
    core::abi::ethereum_types::BigEndianHash,
    providers::Middleware,
    types::{Address, BlockId, Bytes, H160, H256, U256},
    utils::keccak256,
};
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::Stream,
    task::{Context, Poll},
    Future, FutureExt,
};
use revm::{db::DatabaseRef, AccountInfo, Bytecode, KECCAK_EMPTY};
use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    pin::Pin,
    sync::{
        mpsc::{channel as oneshot_channel, Sender as OneshotSender},
        Arc,
    },
};
use tracing::{error, trace, warn};

type AccountFuture<Err> =
    Pin<Box<dyn Future<Output = (Result<(U256, U256, Bytes), Err>, Address)> + Send>>;
type StorageFuture<Err> = Pin<Box<dyn Future<Output = (Result<U256, Err>, Address, U256)> + Send>>;
type BlockHashFuture<Err> = Pin<Box<dyn Future<Output = (Result<H256, Err>, u64)> + Send>>;

/// Request variants that are executed by the provider
enum ProviderRequest<Err> {
    Account(AccountFuture<Err>),
    Storage(StorageFuture<Err>),
    BlockHash(BlockHashFuture<Err>),
}

/// The Request type the Backend listens for
#[derive(Debug)]
enum BackendRequest {
    /// Fetch the account info
    Basic(Address, OneshotSender<AccountInfo>),
    /// Fetch a storage slot
    Storage(Address, U256, OneshotSender<U256>),
    /// Fetch a block hash
    BlockHash(u64, OneshotSender<H256>),
    /// Sets the pinned block to fetch data from
    SetPinnedBlock(BlockId),
}

/// Handles an internal provider and listens for requests.
///
/// This handler will remain active as long as it is reachable (request channel still open) and
/// requests are in progress.
#[must_use = "BackendHandler does nothing unless polled."]
pub struct BackendHandler<M: Middleware> {
    provider: M,
    /// Stores all the data.
    db: BlockchainDb,
    /// Requests currently in progress
    pending_requests: Vec<ProviderRequest<eyre::Error>>,
    /// Listeners that wait for a `get_account` related response
    account_requests: HashMap<Address, Vec<OneshotSender<AccountInfo>>>,
    /// Listeners that wait for a `get_storage_at` response
    storage_requests: HashMap<(Address, U256), Vec<OneshotSender<U256>>>,
    /// Listeners that wait for a `get_block` response
    block_requests: HashMap<u64, Vec<OneshotSender<H256>>>,
    /// Incoming commands.
    incoming: Receiver<BackendRequest>,
    /// unprocessed queued requests
    queued_requests: VecDeque<BackendRequest>,
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
                trace!(target: "backendhandler", "received request basic address={:?}", addr);
                let acc = self.db.accounts().read().get(&addr).cloned();
                if let Some(basic) = acc {
                    let _ = sender.send(basic);
                } else {
                    self.request_account(addr, sender);
                }
            }
            BackendRequest::BlockHash(number, sender) => {
                let hash = self.db.block_hashes().read().get(&number).cloned();
                if let Some(hash) = hash {
                    let _ = sender.send(hash);
                } else {
                    self.request_hash(number, sender);
                }
            }
            BackendRequest::Storage(addr, idx, sender) => {
                // account is already stored in the cache
                let value =
                    self.db.storage().read().get(&addr).and_then(|acc| acc.get(&idx).copied());
                if let Some(value) = value {
                    let _ = sender.send(value);
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
                trace!(target: "backendhandler", "preparing storage request, address={:?}, idx={}", address, idx);
                entry.insert(vec![listener]);
                let provider = self.provider.clone();
                let block_id = self.block_id;
                let fut = Box::pin(async move {
                    // serialize & deserialize back to U256
                    let idx_req = H256::from_uint(&idx);
                    let storage = provider.get_storage_at(address, idx_req, block_id).await;
                    let storage =
                        storage.map(|storage| storage.into_uint()).map_err(|err| eyre::eyre!(err));
                    (storage, address, idx)
                });
                self.pending_requests.push(ProviderRequest::Storage(fut));
            }
        }
    }

    /// returns the future that fetches the account data
    fn get_account_req(&self, address: Address) -> ProviderRequest<eyre::Error> {
        trace!(target: "backendhandler", "preparing account request, address={:?}", address);
        let provider = self.provider.clone();
        let block_id = self.block_id;
        let fut = Box::pin(async move {
            let balance = provider.get_balance(address, block_id);
            let nonce = provider.get_transaction_count(address, block_id);
            let code = provider.get_code(address, block_id);
            let resp = tokio::try_join!(balance, nonce, code).map_err(|err| eyre::eyre!(err));
            (resp, address)
        });
        ProviderRequest::Account(fut)
    }

    /// process a request for an account
    fn request_account(&mut self, address: Address, listener: OneshotSender<AccountInfo>) {
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

    /// process a request for a block hash
    fn request_hash(&mut self, number: u64, listener: OneshotSender<H256>) {
        match self.block_requests.entry(number) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(listener);
            }
            Entry::Vacant(entry) => {
                trace!(target: "backendhandler", "preparing block hash request, number={}", number);
                entry.insert(vec![listener]);
                let provider = self.provider.clone();
                let fut = Box::pin(async move {
                    let block = provider.get_block(number).await;

                    let block_hash = match block {
                        Ok(Some(block)) => Ok(block
                            .hash
                            .expect("empty block hash on mined block, this should never happen")),
                        Ok(None) => {
                            warn!(target: "backendhandler", ?number, "block not found");
                            // if no block was returned then the block does not exist, in which case
                            // we return empty hash
                            Ok(KECCAK_EMPTY)
                        }
                        Err(_) => {
                            error!(target: "backendhandler", ?number, "failed to get block");
                            Err(eyre::eyre!("block {number} not found"))
                        }
                    };
                    (block_hash, number)
                });
                self.pending_requests.push(ProviderRequest::BlockHash(fut));
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
                        return Poll::Ready(())
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
                            let (balance, nonce, code) = resp.unwrap_or_else(|report| {
                                panic!("Failed to get account for {}\n{}", addr, report);
                            });

                            // convert it to revm-style types
                            let (code, code_hash) = if !code.0.is_empty() {
                                (Some(code.0.clone()), keccak256(&code).into())
                            } else {
                                (Some(bytes::Bytes::default()), KECCAK_EMPTY)
                            };

                            // update the cache
                            let acc = AccountInfo {
                                nonce: nonce.as_u64(),
                                balance,
                                code: code.map(|bytes| Bytecode::new_raw(bytes).to_checked()),
                                code_hash,
                            };
                            pin.db.accounts().write().insert(addr, acc.clone());

                            // notify all listeners
                            if let Some(listeners) = pin.account_requests.remove(&addr) {
                                listeners.into_iter().for_each(|l| {
                                    let _ = l.send(acc.clone());
                                })
                            }
                            continue
                        }
                    }
                    ProviderRequest::Storage(fut) => {
                        if let Poll::Ready((resp, addr, idx)) = fut.poll_unpin(cx) {
                            let value = resp.unwrap_or_else(|report| {
                                panic!("Failed to get storage for {} at {}\n{}", addr, idx, report);
                            });

                            // update the cache
                            pin.db.storage().write().entry(addr).or_default().insert(idx, value);

                            // notify all listeners
                            if let Some(listeners) = pin.storage_requests.remove(&(addr, idx)) {
                                listeners.into_iter().for_each(|l| {
                                    let _ = l.send(value);
                                })
                            }
                            continue
                        }
                    }
                    ProviderRequest::BlockHash(fut) => {
                        if let Poll::Ready((block_hash, number)) = fut.poll_unpin(cx) {
                            let value = block_hash.unwrap_or_else(|report| {
                                panic!("Failed to get block hash for {}\n{}", number, report);
                            });

                            // update the cache
                            pin.db.block_hashes().write().insert(number, value);

                            // notify all listeners
                            if let Some(listeners) = pin.block_requests.remove(&number) {
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

            // If no new requests have been queued, break to
            // be polled again later.
            if pin.queued_requests.is_empty() {
                return Poll::Pending
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
/// The `BackendHandler` holds an ethers `Provider` to look up missing accounts or storage slots
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
#[derive(Debug, Clone)]
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
    pub async fn spawn_backend<M>(provider: M, db: BlockchainDb, pin_block: Option<BlockId>) -> Self
    where
        M: Middleware + Unpin + 'static + Clone,
    {
        let (shared, handler) = Self::new(provider, db, pin_block);
        // spawn the provider handler to a task
        trace!(target: "backendhandler", "spawning Backendhandler task");
        tokio::spawn(handler);
        shared
    }

    /// Same as `Self::spawn_backend` but spawns the `BackendHandler` on a separate `std::thread` in
    /// its own `tokio::Runtime`
    pub fn spawn_backend_thread<M>(
        provider: M,
        db: BlockchainDb,
        pin_block: Option<BlockId>,
    ) -> Self
    where
        M: Middleware + Unpin + 'static + Clone,
    {
        let (shared, handler) = Self::new(provider, db, pin_block);

        // spawn a light-weight thread with a thread-local async runtime just for
        // sending and receiving data from the remote client
        let _ = std::thread::Builder::new()
            .name("fork-backend-thread".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create fork-backend-thread tokio runtime");

                rt.block_on(async move { handler.await });
            })
            .expect("failed to spawn backendhandler thread");
        trace!(target: "backendhandler", "spawned Backendhandler thread");

        shared
    }

    /// Returns a new `SharedBackend` and the `BackendHandler`
    pub fn new<M>(
        provider: M,
        db: BlockchainDb,
        pin_block: Option<BlockId>,
    ) -> (Self, BackendHandler<M>)
    where
        M: Middleware + Unpin + 'static + Clone,
    {
        let (backend, backend_rx) = channel(1);
        let cache = Arc::new(FlushJsonBlockCacheDB(Arc::clone(db.cache())));
        let handler = BackendHandler::new(provider, db, backend_rx, pin_block);
        (Self { backend, cache }, handler)
    }

    /// Updates the pinned block to fetch data from
    pub fn set_pinned_block(&self, block: impl Into<BlockId>) -> eyre::Result<()> {
        tokio::task::block_in_place(|| {
            let req = BackendRequest::SetPinnedBlock(block.into());
            self.backend.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))
        })
    }

    fn do_get_basic(&self, address: Address) -> eyre::Result<AccountInfo> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Basic(address, sender);
            self.backend.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
            Ok(rx.recv()?)
        })
    }

    fn do_get_storage(&self, address: Address, index: U256) -> eyre::Result<U256> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::Storage(address, index, sender);
            self.backend.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
            Ok(rx.recv()?)
        })
    }

    fn do_get_block_hash(&self, number: u64) -> eyre::Result<H256> {
        tokio::task::block_in_place(|| {
            let (sender, rx) = oneshot_channel();
            let req = BackendRequest::BlockHash(number, sender);
            self.backend.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
            Ok(rx.recv()?)
        })
    }

    /// Flushes the DB to disk if caching is enabled
    pub(crate) fn flush_cache(&self) {
        self.cache.0.flush();
    }
}

impl DatabaseRef for SharedBackend {
    fn basic(&self, address: H160) -> AccountInfo {
        trace!( target: "sharedbackend", "request basic {:?}", address);
        self.do_get_basic(address).unwrap_or_else(|_| {
            panic!("Failed to send/recv `basic` for {address}");
        })
    }

    fn code_by_hash(&self, _address: H256) -> Bytecode {
        panic!("Should not be called. Code is already loaded.")
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        trace!( target: "sharedbackend", "request storage {:?} at {:?}", address, index);
        self.do_get_storage(address, index).unwrap_or_else(|_| {
            panic!("Failed to send/recv `storage` for {address} at {index}");
        })
    }

    fn block_hash(&self, number: U256) -> H256 {
        if number > U256::from(u64::MAX) {
            return KECCAK_EMPTY
        }
        let number = number.as_u64();
        trace!( target: "sharedbackend", "request block hash for number {:?}", number);
        self.do_get_block_hash(number).unwrap_or_else(|_| {
            panic!("Failed to send/recv `block_hash` for {number}");
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::executor::{
        fork::{BlockchainDbMeta, JsonBlockCacheDB},
        opts::EvmOpts,
        Backend,
    };
    use ethers::{
        providers::{Http, Provider},
        solc::utils::RuntimeOrHandle,
        types::Address,
    };

    use crate::executor::fork::CreateFork;
    use ethers::types::Chain;
    use foundry_config::Config;
    use std::{collections::BTreeSet, convert::TryFrom, path::PathBuf, sync::Arc};

    use super::*;
    const ENDPOINT: &str = "https://mainnet.infura.io/v3/c60b0bb42f8a4c6481ecd229eddaca27";

    #[test]
    fn shared_backend() {
        let provider = Provider::<Http>::try_from(ENDPOINT).unwrap();
        let meta = BlockchainDbMeta {
            cfg_env: Default::default(),
            block_env: Default::default(),
            hosts: BTreeSet::from([ENDPOINT.to_string()]),
        };

        let db = BlockchainDb::new(meta, None);
        let runtime = RuntimeOrHandle::new();
        let backend =
            runtime.block_on(SharedBackend::spawn_backend(Arc::new(provider), db.clone(), None));

        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let idx = U256::from(0u64);
        let value = backend.storage(address, idx);
        let account = backend.basic(address);

        let mem_acc = db.accounts().read().get(&address).unwrap().clone();
        assert_eq!(account.balance, mem_acc.balance);
        assert_eq!(account.nonce, mem_acc.nonce);
        let slots = db.storage().read().get(&address).unwrap().clone();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots.get(&idx).copied().unwrap(), value);

        let num = U256::from(10u64);
        let hash = backend.block_hash(num);
        let mem_hash = *db.block_hashes().read().get(&num.as_u64()).unwrap();
        assert_eq!(hash, mem_hash);

        let max_slots = 5;
        let handle = std::thread::spawn(move || {
            for i in 1..max_slots {
                let idx = U256::from(i);
                let _ = backend.storage(address, idx);
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
        let provider = Provider::<Http>::try_from(ENDPOINT).unwrap();

        let block_num = provider.get_block_number().await.unwrap().as_u64();

        let config = Config::figment();
        let mut evm_opts = config.extract::<EvmOpts>().unwrap();
        evm_opts.fork_block_number = Some(block_num);

        let env = evm_opts.fork_evm_env(ENDPOINT).await.unwrap();

        let fork = CreateFork {
            enable_caching: true,
            url: ENDPOINT.to_string(),
            env: env.clone(),
            evm_opts,
        };

        let backend = Backend::spawn(Some(fork));

        // some rng contract from etherscan
        let address: Address = "63091244180ae240c87d1f528f5f269134cb07b3".parse().unwrap();

        let idx = U256::from(0u64);
        let _value = backend.storage(address, idx);
        let _account = backend.basic(address);

        // fill some slots
        let num_slots = 10u64;
        for idx in 1..num_slots {
            let _ = backend.storage(address, idx.into());
        }
        drop(backend);

        let meta =
            BlockchainDbMeta { cfg_env: env.cfg, block_env: env.block, hosts: Default::default() };

        let db = BlockchainDb::new(
            meta,
            Some(Config::foundry_block_cache_dir(Chain::Mainnet, block_num).unwrap()),
        );
        assert!(db.accounts().read().contains_key(&address));
        assert!(db.storage().read().contains_key(&address));
        assert_eq!(db.storage().read().get(&address).unwrap().len(), num_slots as usize);
    }
}
