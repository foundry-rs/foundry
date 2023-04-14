//! Support for running multiple fork backend
//!
//! The design is similar to the single `SharedBackend`, `BackendHandler` but supports multiple
//! concurrently active pairs at once.

use crate::{
    executor::fork::{BackendHandler, BlockchainDb, BlockchainDbMeta, CreateFork, SharedBackend},
    utils::ru256_to_u256,
};
use ethers::{
    abi::{AbiDecode, AbiEncode, AbiError},
    providers::{Http, Provider, RetryClient},
    types::{BlockId, BlockNumber},
};
use foundry_common::ProviderBuilder;
use foundry_config::Config;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::{Fuse, Stream},
    task::{Context, Poll},
    Future, FutureExt, StreamExt,
};
use revm::primitives::Env;
use std::{
    collections::HashMap,
    fmt,
    pin::Pin,
    sync::{
        mpsc::{channel as oneshot_channel, Sender as OneshotSender},
        Arc,
    },
    time::Duration,
};
use tracing::trace;

/// The identifier for a specific fork, this could be the name of the network a custom descriptive
/// name.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ForkId(pub String);

impl fmt::Display for ForkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: Into<String>> From<T> for ForkId {
    fn from(id: T) -> Self {
        Self(id.into())
    }
}

impl AbiEncode for ForkId {
    fn encode(self) -> Vec<u8> {
        AbiEncode::encode(self.0)
    }
}

impl AbiDecode for ForkId {
    fn decode(bytes: impl AsRef<[u8]>) -> Result<Self, AbiError> {
        Ok(Self(String::decode(bytes)?))
    }
}

/// The Sender half of multi fork pair.
/// Can send requests to the `MultiForkHandler` to create forks
#[derive(Debug, Clone)]
pub struct MultiFork {
    /// Channel to send `Request`s to the handler
    handler: Sender<Request>,
    /// Ensures that all rpc resources get flushed properly
    _shutdown: Arc<ShutDownMultiFork>,
}

// === impl MultiForkBackend ===

impl MultiFork {
    /// Creates a new pair multi fork pair
    pub fn new() -> (Self, MultiForkHandler) {
        let (handler, handler_rx) = channel(1);
        let _shutdown = Arc::new(ShutDownMultiFork { handler: Some(handler.clone()) });
        (Self { handler, _shutdown }, MultiForkHandler::new(handler_rx))
    }

    /// Creates a new pair and spawns the `MultiForkHandler` on a background thread
    ///
    /// Also returns the `JoinHandle` of the spawned thread.
    pub fn spawn() -> Self {
        trace!(target: "fork::multi", "spawning multifork");

        let (fork, mut handler) = Self::new();
        // spawn a light-weight thread with a thread-local async runtime just for
        // sending and receiving data from the remote client(s)
        let _ = std::thread::Builder::new()
            .name("multi-fork-backend-thread".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create multi-fork-backend-thread tokio runtime");

                rt.block_on(async move {
                    // flush cache every 60s, this ensures that long-running fork tests get their
                    // cache flushed from time to time
                    // NOTE: we install the interval here because the `tokio::timer::Interval`
                    // requires a rt
                    handler.set_flush_cache_interval(Duration::from_secs(60));
                    handler.await
                });
            })
            .expect("failed to spawn multi fork handler thread");
        trace!(target: "fork::multi", "spawned MultiForkHandler thread");
        fork
    }

    /// Returns a fork backend
    ///
    /// If no matching fork backend exists it will be created
    pub fn create_fork(&self, fork: CreateFork) -> eyre::Result<(ForkId, SharedBackend, Env)> {
        trace!("Creating new fork, url={}, block={:?}", fork.url, fork.evm_opts.fork_block_number);
        let (sender, rx) = oneshot_channel();
        let req = Request::CreateFork(Box::new(fork), sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        rx.recv()?
    }

    /// Rolls the block of the fork
    ///
    /// If no matching fork backend exists it will be created
    pub fn roll_fork(
        &self,
        fork: ForkId,
        block: u64,
    ) -> eyre::Result<(ForkId, SharedBackend, Env)> {
        trace!(?fork, ?block, "rolling fork");
        let (sender, rx) = oneshot_channel();
        let req = Request::RollFork(fork, block, sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        rx.recv()?
    }

    /// Returns the `Env` of the given fork, if any
    pub fn get_env(&self, fork: ForkId) -> eyre::Result<Option<Env>> {
        trace!(?fork, "getting env config");
        let (sender, rx) = oneshot_channel();
        let req = Request::GetEnv(fork, sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }

    /// Returns the corresponding fork if it exists
    ///
    /// Returns `None` if no matching fork backend is available.
    pub fn get_fork(&self, id: impl Into<ForkId>) -> eyre::Result<Option<SharedBackend>> {
        let id = id.into();
        trace!(?id, "get fork backend");
        let (sender, rx) = oneshot_channel();
        let req = Request::GetFork(id, sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }

    /// Returns the corresponding fork url if it exists
    ///
    /// Returns `None` if no matching fork is available.
    pub fn get_fork_url(&self, id: impl Into<ForkId>) -> eyre::Result<Option<String>> {
        let (sender, rx) = oneshot_channel();
        let req = Request::GetForkUrl(id.into(), sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }
}

type Handler = BackendHandler<Arc<Provider<RetryClient<Http>>>>;

type CreateFuture = Pin<Box<dyn Future<Output = eyre::Result<(CreatedFork, Handler)>> + Send>>;
type CreateSender = OneshotSender<eyre::Result<(ForkId, SharedBackend, Env)>>;
type GetEnvSender = OneshotSender<Option<Env>>;

/// Request that's send to the handler
#[derive(Debug)]
enum Request {
    /// Creates a new ForkBackend
    CreateFork(Box<CreateFork>, CreateSender),
    /// Returns the Fork backend for the `ForkId` if it exists
    GetFork(ForkId, OneshotSender<Option<SharedBackend>>),
    /// Adjusts the block that's being forked
    RollFork(ForkId, u64, CreateSender),
    /// Returns the environment of the fork
    GetEnv(ForkId, GetEnvSender),
    /// Shutdowns the entire `MultiForkHandler`, see `ShutDownMultiFork`
    ShutDown(OneshotSender<()>),
    /// Returns the Fork Url for the `ForkId` if it exists
    GetForkUrl(ForkId, OneshotSender<Option<String>>),
}

enum ForkTask {
    /// Contains the future that will establish a new fork
    Create(CreateFuture, ForkId, CreateSender, Vec<CreateSender>),
}

/// The type that manages connections in the background
#[must_use = "MultiForkHandler does nothing unless polled."]
pub struct MultiForkHandler {
    /// Incoming requests from the `MultiFork`.
    incoming: Fuse<Receiver<Request>>,

    /// All active handlers
    ///
    /// It's expected that this list will be rather small (<10)
    handlers: Vec<(ForkId, Handler)>,

    // tasks currently in progress
    pending_tasks: Vec<ForkTask>,

    /// All created Forks in order to reuse them
    forks: HashMap<ForkId, CreatedFork>,

    /// The retries to allow for new providers
    retries: u32,

    /// Initial backoff delay for requests
    backoff: u64,

    /// Optional periodic interval to flush rpc cache
    flush_cache_interval: Option<tokio::time::Interval>,
}

// === impl MultiForkHandler ===

impl MultiForkHandler {
    fn new(incoming: Receiver<Request>) -> Self {
        Self {
            incoming: incoming.fuse(),
            handlers: Default::default(),
            pending_tasks: Default::default(),
            forks: Default::default(),
            retries: 8,
            // 800ms
            backoff: 800,
            flush_cache_interval: None,
        }
    }

    /// Sets the interval after which all rpc caches should be flushed periodically
    pub fn set_flush_cache_interval(&mut self, period: Duration) -> &mut Self {
        self.flush_cache_interval =
            Some(tokio::time::interval_at(tokio::time::Instant::now() + period, period));
        self
    }

    /// Returns the list of additional senders of a matching task for the given id, if any.
    fn find_in_progress_task(&mut self, id: &ForkId) -> Option<&mut Vec<CreateSender>> {
        for task in self.pending_tasks.iter_mut() {
            #[allow(irrefutable_let_patterns)]
            if let ForkTask::Create(_, in_progress, _, additional) = task {
                if in_progress == id {
                    return Some(additional)
                }
            }
        }
        None
    }

    fn create_fork(&mut self, fork: CreateFork, sender: CreateSender) {
        let fork_id = create_fork_id(&fork.url, fork.evm_opts.fork_block_number);
        trace!(?fork_id, "created new forkId");

        if let Some(fork) = self.forks.get_mut(&fork_id) {
            fork.num_senders += 1;
            let _ = sender.send(Ok((fork_id, fork.backend.clone(), fork.opts.env.clone())));
        } else {
            // there could already be a task for the requested fork in progress
            if let Some(in_progress) = self.find_in_progress_task(&fork_id) {
                in_progress.push(sender);
                return
            }

            let retries = self.retries;
            let backoff = self.backoff;
            // need to create a new fork
            let task = Box::pin(create_fork(fork, retries, backoff));
            self.pending_tasks.push(ForkTask::Create(task, fork_id, sender, Vec::new()));
        }
    }

    fn on_request(&mut self, req: Request) {
        match req {
            Request::CreateFork(fork, sender) => self.create_fork(*fork, sender),
            Request::GetFork(fork_id, sender) => {
                let fork = self.forks.get(&fork_id).map(|f| f.backend.clone());
                let _ = sender.send(fork);
            }
            Request::RollFork(fork_id, block, sender) => {
                if let Some(fork) = self.forks.get(&fork_id) {
                    trace!(target: "fork::multi", "rolling {} to {}", fork_id, block);
                    let mut opts = fork.opts.clone();
                    opts.evm_opts.fork_block_number = Some(block);
                    self.create_fork(opts, sender)
                } else {
                    let _ = sender.send(Err(eyre::eyre!("No matching fork exits for {}", fork_id)));
                }
            }
            Request::GetEnv(fork_id, sender) => {
                let _ = sender.send(self.forks.get(&fork_id).map(|fork| fork.opts.env.clone()));
            }
            Request::ShutDown(sender) => {
                trace!(target: "fork::multi", "received shutdown signal");
                // we're emptying all fork backends, this way we ensure all caches get flushed
                self.forks.clear();
                self.handlers.clear();
                let _ = sender.send(());
            }
            Request::GetForkUrl(fork_id, sender) => {
                let fork = self.forks.get(&fork_id).map(|f| f.opts.url.clone());
                let _ = sender.send(fork);
            }
        }
    }
}

// Drives all handler to completion
// This future will finish once all underlying BackendHandler are completed
impl Future for MultiForkHandler {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // receive new requests
        loop {
            match Pin::new(&mut pin.incoming).poll_next(cx) {
                Poll::Ready(Some(req)) => {
                    pin.on_request(req);
                }
                Poll::Ready(None) => {
                    // channel closed, but we still need to drive the fork handlers to completion
                    trace!(target: "fork::multi", "request channel closed");
                    break
                }
                Poll::Pending => break,
            }
        }

        // advance all tasks
        for n in (0..pin.pending_tasks.len()).rev() {
            let task = pin.pending_tasks.swap_remove(n);
            match task {
                ForkTask::Create(mut fut, id, sender, additional_senders) => {
                    if let Poll::Ready(resp) = fut.poll_unpin(cx) {
                        match resp {
                            Ok((fork, handler)) => {
                                pin.handlers.push((id.clone(), handler));
                                let backend = fork.backend.clone();
                                let env = fork.opts.env.clone();
                                pin.forks.insert(id.clone(), fork);

                                let _ = sender.send(Ok((id.clone(), backend.clone(), env.clone())));

                                // also notify all additional senders
                                for sender in additional_senders {
                                    let _ =
                                        sender.send(Ok((id.clone(), backend.clone(), env.clone())));
                                }
                            }
                            Err(err) => {
                                let _ = sender.send(Err(eyre::eyre!("{err}")));
                                for sender in additional_senders {
                                    let _ = sender.send(Err(eyre::eyre!("{err}")));
                                }
                            }
                        }
                    } else {
                        pin.pending_tasks.push(ForkTask::Create(
                            fut,
                            id,
                            sender,
                            additional_senders,
                        ));
                    }
                }
            }
        }

        // advance all handlers
        for n in (0..pin.handlers.len()).rev() {
            let (id, mut handler) = pin.handlers.swap_remove(n);
            match handler.poll_unpin(cx) {
                Poll::Ready(_) => {
                    trace!(target: "fork::multi", "fork {:?} completed", id);
                }
                Poll::Pending => {
                    pin.handlers.push((id, handler));
                }
            }
        }

        if pin.handlers.is_empty() && pin.incoming.is_done() {
            trace!(target: "fork::multi", "completed");
            return Poll::Ready(())
        }

        // periodically flush cached RPC state
        if pin
            .flush_cache_interval
            .as_mut()
            .map(|interval| interval.poll_tick(cx).is_ready())
            .unwrap_or_default() &&
            !pin.forks.is_empty()
        {
            trace!(target: "fork::multi", "tick flushing caches");
            let forks = pin.forks.values().map(|f| f.backend.clone()).collect::<Vec<_>>();
            // flush this on new thread to not block here
            std::thread::spawn(move || {
                forks.into_iter().for_each(|fork| fork.flush_cache());
            });
        }

        Poll::Pending
    }
}

/// Tracks the created Fork
#[derive(Debug)]
struct CreatedFork {
    /// How the fork was initially created
    opts: CreateFork,
    /// Copy of the sender
    backend: SharedBackend,
    /// How many consumers there are, since a `SharedBacked` can be used by multiple
    /// consumers
    num_senders: usize,
}

// === impl CreatedFork ===

impl CreatedFork {
    pub fn new(opts: CreateFork, backend: SharedBackend) -> Self {
        Self { opts, backend, num_senders: 1 }
    }
}

/// A type that's used to signaling the `MultiForkHandler` when it's time to shut down.
///
/// This is essentially a sync on drop, so that the `MultiForkHandler` can flush all rpc cashes
///
/// This type intentionally does not implement `Clone` since it's intended that there's only once
/// instance.
#[derive(Debug)]
struct ShutDownMultiFork {
    handler: Option<Sender<Request>>,
}

impl Drop for ShutDownMultiFork {
    fn drop(&mut self) {
        trace!(target: "fork::multi", "initiating shutdown");
        let (sender, rx) = oneshot_channel();
        let req = Request::ShutDown(sender);
        if let Some(mut handler) = self.handler.take() {
            if handler.try_send(req).is_ok() {
                let _ = rx.recv();
                trace!(target: "fork::cache", "multifork backend shutdown");
            }
        }
    }
}

/// Returns  the identifier for a Fork which consists of the url and the block number
fn create_fork_id(url: &str, num: Option<u64>) -> ForkId {
    let num = num.map(|num| BlockNumber::Number(num.into())).unwrap_or(BlockNumber::Latest);
    ForkId(format!("{url}@{num}"))
}

/// Creates a new fork
///
/// This will establish a new `Provider` to the endpoint and return the Fork Backend
async fn create_fork(
    mut fork: CreateFork,
    retries: u32,
    backoff: u64,
) -> eyre::Result<(CreatedFork, Handler)> {
    let provider = Arc::new(
        ProviderBuilder::new(fork.url.as_str())
            .max_retry(retries)
            .initial_backoff(backoff)
            .compute_units_per_second(fork.evm_opts.get_compute_units_per_second())
            .build()?,
    );

    // initialise the fork environment
    let (env, block) = fork.evm_opts.fork_evm_env(&fork.url).await?;
    fork.env = env;
    let meta = BlockchainDbMeta::new(fork.env.clone(), fork.url.clone());
    let number = ru256_to_u256(meta.block_env.number).as_u64();

    // determine the cache path if caching is enabled
    let cache_path = if fork.enable_caching {
        Config::foundry_block_cache_dir(ru256_to_u256(meta.cfg_env.chain_id).as_u64(), number)
    } else {
        None
    };

    let db = BlockchainDb::new(meta, cache_path);
    let (backend, handler) =
        SharedBackend::new(provider, db, Some(BlockId::Number(BlockNumber::Number(number.into()))));
    let fork = CreatedFork::new(fork, backend);
    Ok((fork, handler))
}
