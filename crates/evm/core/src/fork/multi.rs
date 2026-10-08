//! Support for running multiple fork backends.
//!
//! The design is similar to the single `SharedBackend`, `BackendHandler` but supports multiple
//! concurrently active pairs at once.

use super::{CreateFork, Fork, bal};
use crate::{FoundryBlock, opts::ForkContext};
use alloy_eips::{BlockNumHash, eip7928::BlockAccessList};
use alloy_evm::EvmEnv;
use alloy_network::{AnyNetwork, Network};
use alloy_primitives::map::HashMap;
use foundry_config::Config;
use foundry_fork_db::{
    BackendHandler, BlockchainDb, ForkBlock, ForkBlockEnv, SharedBackend, cache::BlockchainDbMeta,
};
use futures::{
    FutureExt, StreamExt,
    channel::mpsc::{Receiver, Sender, channel},
    stream::Fuse,
    task::{Context, Poll},
};
use revm::primitives::hardfork::SpecId;
use std::{
    fmt::{self, Write},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Sender as OneshotSender, channel as oneshot_channel},
    },
    time::Duration,
};

/// The _unique_ identifier for a specific fork, this could be the name of the network a custom
/// descriptive name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ForkId(pub String);

impl ForkId {
    /// Returns the identifier for a Fork from a URL and block number.
    pub fn new(url: &str, num: Option<u64>) -> Self {
        Self::new_with_context(url, num, None)
    }

    fn new_with_context(
        url: &str,
        num: Option<u64>,
        context: Option<&crate::opts::ForkContext>,
    ) -> Self {
        let mut id = url.to_string();
        if let Some(context) = context {
            write!(
                id,
                "#{}:{}:{}:{}:{:?}:{:?}:{:?}:{:?}",
                context.execution_chain_id,
                context.source_chain_id,
                context.network,
                context.network_profile.execution_profile_name(),
                context.hardfork,
                context.instance_id,
                context.source_fork_block_number,
                context.source_fork_block_hash
            )
            .unwrap();
        }
        id.push('@');
        match num {
            Some(n) => write!(id, "{n:#x}").unwrap(),
            None => id.push_str("latest"),
        }
        Self(id)
    }

    /// Returns the identifier for an exactly resolved fork.
    fn resolved(url: &str, fork: &Fork) -> Self {
        let mut id = Self::exact(url, fork, fork.block());
        if fork.state_by_number {
            id.0.push_str("#state-by-number");
        }
        id
    }

    /// Exact rolls always read state by hash, regardless of the parent fork's state mode.
    fn exact(url: &str, fork: &Fork, block: BlockNumHash) -> Self {
        let mut id = Self::new_with_context(url, Some(block.number), Some(&fork.context())).0;
        write!(id, "#{}:{}", block.hash, fork.source_id()).unwrap();
        Self(id)
    }

    /// Returns the identifier of the fork.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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

/// Backend, environment, and identity returned after creating or rolling a fork.
pub struct ForkResult<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    /// Identifier assigned to the fork.
    pub id: ForkId,
    /// Backend pinned to the resolved fork block.
    pub backend: SharedBackend<N, BLOCK>,
    /// EVM environment reconstructed from the resolved fork block.
    pub env: EvmEnv<SPEC, BLOCK>,
    /// Exact source and block identity used to construct the backend.
    pub fork: Fork,
}

/// The Sender half of multi fork pair.
/// Can send requests to the `MultiForkHandler` to create forks.
#[derive(Clone, Debug)]
#[must_use]
pub struct MultiFork<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    /// Channel to send `Request`s to the handler.
    handler: Sender<Request<N, SPEC, BLOCK>>,
    /// Ensures that all rpc resources get flushed properly.
    _shutdown: Arc<ShutDownMultiFork<N, SPEC, BLOCK>>,
}

impl<
    N: Network,
    SPEC: Into<SpecId> + Default + Copy + Unpin + Send + 'static,
    BLOCK: FoundryBlock + ForkBlockEnv + Default + Unpin,
> MultiFork<N, SPEC, BLOCK>
{
    /// Creates a new pair and spawns the `MultiForkHandler` on a background thread.
    pub fn spawn() -> Self {
        Self::spawn_with_forks(HashMap::default(), None)
    }

    /// Starts an independent registry sharing the existing remote caches.
    pub(crate) fn scoped(&self) -> eyre::Result<Self> {
        let (sender, rx) = oneshot_channel();
        self.handler
            .clone()
            .try_send(Request::CloneForks(sender))
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        Ok(Self::spawn_with_forks(rx.recv()?, Some(self._shutdown.clone())))
    }

    fn spawn_with_forks(
        forks: HashMap<ForkId, CreatedFork<N, SPEC, BLOCK>>,
        parent: Option<Arc<ShutDownMultiFork<N, SPEC, BLOCK>>>,
    ) -> Self {
        trace!(target: "fork::multi", "spawning multifork");
        let (mut fork, mut handler) = Self::new();
        handler.forks = forks;
        Arc::get_mut(&mut fork._shutdown).unwrap()._parent = parent;

        // Spawn a light-weight thread just for sending and receiving data from the remote
        // client(s).
        let fut = async move {
            // Flush cache every 60s, this ensures that long-running fork tests get their
            // cache flushed from time to time.
            // NOTE: we install the interval here because the `tokio::timer::Interval`
            // requires a rt.
            handler.set_flush_cache_interval(Duration::from_secs(60));
            handler.await
        };
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => _ = rt.spawn(fut),
            Err(_) => {
                trace!(target: "fork::multi", "spawning multifork backend thread");
                _ = std::thread::Builder::new()
                    .name("multi-fork-backend".into())
                    .spawn(move || {
                        tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("failed to build tokio runtime")
                            .block_on(fut)
                    })
                    .expect("failed to spawn thread")
            }
        }

        trace!(target: "fork::multi", "spawned MultiForkHandler thread");
        fork
    }

    /// Creates a new pair multi fork pair.
    ///
    /// Use [`spawn`](Self::spawn) instead.
    #[doc(hidden)]
    pub fn new() -> (Self, MultiForkHandler<N, SPEC, BLOCK>) {
        let (handler, handler_rx) = channel(1);
        let _shutdown =
            Arc::new(ShutDownMultiFork { handler: Some(handler.clone()), _parent: None });
        (Self { handler, _shutdown }, MultiForkHandler::new(handler_rx))
    }

    /// Returns a fork backend.
    ///
    /// If no matching fork backend exists it will be created.
    pub fn create_fork(&self, fork: CreateFork) -> eyre::Result<ForkResult<N, SPEC, BLOCK>> {
        trace!("Creating new fork, url={}, block={:?}", fork.url, fork.evm_opts.fork_block_number);
        let (sender, rx) = oneshot_channel();
        let req = Request::CreateFork(Box::new(fork), sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        rx.recv()?
    }

    /// Rolls the block of the fork.
    ///
    /// If no matching fork backend exists it will be created.
    pub fn roll_fork(&self, fork: ForkId, block: u64) -> eyre::Result<ForkResult<N, SPEC, BLOCK>> {
        trace!(?fork, ?block, "rolling fork");
        let (sender, rx) = oneshot_channel();
        let req = Request::RollFork(fork, block, sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        rx.recv()?
    }

    /// Rolls a fork to an already resolved exact block.
    pub fn roll_fork_exact(
        &self,
        fork: ForkId,
        block: BlockNumHash,
    ) -> eyre::Result<ForkResult<N, SPEC, BLOCK>> {
        self.roll_fork_exact_with_bal(fork, block, false)
    }

    /// Rolls to an exact parent block, optionally warming its cache before transaction replay.
    pub(crate) fn roll_fork_exact_with_bal(
        &self,
        fork: ForkId,
        block: BlockNumHash,
        prewarm_bal: bool,
    ) -> eyre::Result<ForkResult<N, SPEC, BLOCK>> {
        trace!(?fork, ?block, "rolling fork to exact block");
        let (sender, rx) = oneshot_channel();
        let req = Request::RollForkExact(fork, block, prewarm_bal, sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        rx.recv()?
    }

    /// Returns the `EvmEnv` of the given fork, if any.
    pub fn get_evm_env(&self, fork: ForkId) -> eyre::Result<Option<EvmEnv<SPEC, BLOCK>>> {
        trace!(?fork, "getting env config");
        let (sender, rx) = oneshot_channel();
        let req = Request::GetEvmEnv(fork, sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }

    /// Returns the corresponding fork if it exists.
    ///
    /// Returns `None` if no matching fork backend is available.
    pub fn get_fork(&self, id: impl Into<ForkId>) -> eyre::Result<Option<SharedBackend<N, BLOCK>>> {
        let id = id.into();
        trace!(?id, "get fork backend");
        let (sender, rx) = oneshot_channel();
        let req = Request::GetFork(id, sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }

    /// Returns the immutable remote fork owned by the selected backend.
    pub fn get_fork_info(&self, id: impl Into<ForkId>) -> eyre::Result<Option<Fork>> {
        let (sender, rx) = oneshot_channel();
        self.handler
            .clone()
            .try_send(Request::GetForkInfo(id.into(), sender))
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        Ok(rx.recv()?)
    }

    /// Returns the corresponding fork url if it exists.
    ///
    /// Returns `None` if no matching fork is available.
    pub fn get_fork_url(&self, id: impl Into<ForkId>) -> eyre::Result<Option<String>> {
        let (sender, rx) = oneshot_channel();
        let req = Request::GetForkUrl(id.into(), sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }

    /// Returns the options used to create the corresponding fork if it exists.
    ///
    /// Returns `None` if no matching fork is available.
    pub fn get_fork_options(&self, id: impl Into<ForkId>) -> eyre::Result<Option<CreateFork>> {
        let (sender, rx) = oneshot_channel();
        let req = Request::GetForkOptions(id.into(), sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }
}

type CreateFuture<SPEC, BLOCK> = Pin<
    Box<
        dyn Future<
                Output = eyre::Result<(
                    ForkId,
                    CreateFork,
                    Fork,
                    EvmEnv<SPEC, BLOCK>,
                    Option<BlockAccessList>,
                )>,
            > + Send,
    >,
>;
type CreateSender<N, SPEC, BLOCK> = OneshotSender<eyre::Result<ForkResult<N, SPEC, BLOCK>>>;
type GetEvmEnvSender<SPEC, BLOCK> = OneshotSender<Option<EvmEnv<SPEC, BLOCK>>>;

/// Request that's send to the handler.
#[derive(Debug)]
enum Request<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    CloneForks(OneshotSender<HashMap<ForkId, CreatedFork<N, SPEC, BLOCK>>>),
    /// Creates a new ForkBackend.
    CreateFork(Box<CreateFork>, CreateSender<N, SPEC, BLOCK>),
    /// Returns the Fork backend for the `ForkId` if it exists.
    GetFork(ForkId, OneshotSender<Option<SharedBackend<N, BLOCK>>>),
    /// Adjusts the block that's being forked, by creating a new fork at the new block.
    RollFork(ForkId, u64, CreateSender<N, SPEC, BLOCK>),
    /// Adjusts the fork to an already resolved exact block.
    RollForkExact(ForkId, BlockNumHash, bool, CreateSender<N, SPEC, BLOCK>),
    /// Returns the environment of the fork.
    GetEvmEnv(ForkId, GetEvmEnvSender<SPEC, BLOCK>),
    /// Shutdowns the entire `MultiForkHandler`, see `ShutDownMultiFork`
    ShutDown(OneshotSender<()>),
    /// Returns the Fork Url for the `ForkId` if it exists.
    GetForkUrl(ForkId, OneshotSender<Option<String>>),
    GetForkInfo(ForkId, OneshotSender<Option<Fork>>),
    /// Returns the options used to create the `ForkId` if it exists.
    GetForkOptions(ForkId, OneshotSender<Option<CreateFork>>),
}

enum ForkTask<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    /// Contains the future that will establish a new fork.
    Create {
        future: CreateFuture<SPEC, BLOCK>,
        id: ForkId,
        prewarm_bal: bool,
        no_fork_bal: bool,
        sender: CreateSender<N, SPEC, BLOCK>,
        additional_senders: Vec<CreateSender<N, SPEC, BLOCK>>,
    },
}

/// The type that manages connections in the background.
#[must_use = "futures do nothing unless polled"]
pub struct MultiForkHandler<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    /// Incoming requests from the `MultiFork`.
    incoming: Fuse<Receiver<Request<N, SPEC, BLOCK>>>,

    /// All active handlers.
    ///
    /// It's expected that this list will be rather small (<10).
    handlers: Vec<(ForkId, BackendHandler<N, BLOCK>)>,

    // tasks currently in progress
    pending_tasks: Vec<ForkTask<N, SPEC, BLOCK>>,

    /// All _unique_ forkids mapped to their corresponding backend.
    ///
    /// Note: The backend can be shared by multiple ForkIds if the target the same provider and
    /// block number.
    forks: HashMap<ForkId, CreatedFork<N, SPEC, BLOCK>>,

    /// Optional periodic interval to flush rpc cache.
    flush_cache_interval: Option<tokio::time::Interval>,
}

impl<
    N: Network,
    SPEC: Into<SpecId> + Default + Copy + Send + 'static,
    BLOCK: FoundryBlock + ForkBlockEnv + Default,
> MultiForkHandler<N, SPEC, BLOCK>
{
    fn new(incoming: Receiver<Request<N, SPEC, BLOCK>>) -> Self {
        Self {
            incoming: incoming.fuse(),
            handlers: Default::default(),
            pending_tasks: Default::default(),
            forks: Default::default(),
            flush_cache_interval: None,
        }
    }

    /// Sets the interval after which all rpc caches should be flushed periodically.
    pub fn set_flush_cache_interval(&mut self, period: Duration) -> &mut Self {
        self.flush_cache_interval =
            Some(tokio::time::interval_at(tokio::time::Instant::now() + period, period));
        self
    }

    /// Returns the list of additional senders of a matching task for the given id, if any.
    fn find_in_progress_task(
        &mut self,
        id: &ForkId,
        prewarm_bal: bool,
        no_fork_bal: bool,
    ) -> Option<&mut Vec<CreateSender<N, SPEC, BLOCK>>> {
        for ForkTask::Create {
            id: in_progress,
            prewarm_bal: pending_prewarm,
            no_fork_bal: pending_opt_out,
            additional_senders,
            ..
        } in &mut self.pending_tasks
        {
            if in_progress == id
                && *pending_prewarm == prewarm_bal
                && *pending_opt_out == no_fork_bal
            {
                return Some(additional_senders);
            }
        }
        None
    }

    fn create_fork(&mut self, fork: CreateFork, sender: CreateSender<N, SPEC, BLOCK>) {
        self.create_fork_with_identity(fork, None, false, None, sender);
    }

    fn create_fork_with_identity(
        &mut self,
        fork: CreateFork,
        expected_identity: Option<ForkContext>,
        prewarm_bal: bool,
        exact_block: Option<(Fork, BlockNumHash)>,
        sender: CreateSender<N, SPEC, BLOCK>,
    ) {
        let no_fork_bal = fork.evm_opts.no_fork_bal;
        let prewarm_bal = prewarm_bal && !no_fork_bal;
        let resolved_id =
            exact_block.as_ref().map(|(parent, block)| ForkId::exact(&fork.url, parent, *block));
        trace!(?resolved_id, "creating fork");

        // Only deduplicate requests that already carry an exact identity. Unresolved requests at
        // the same URL and height can resolve to different blocks across a reorganization.
        if let Some(fork_id) = &resolved_id
            && let Some(in_progress) = self.find_in_progress_task(fork_id, prewarm_bal, no_fork_bal)
        {
            in_progress.push(sender);
            return;
        }

        let already_prewarmed = resolved_id
            .as_ref()
            .and_then(|id| self.forks.get(id))
            .is_some_and(|fork| fork.bal_prewarmed.load(Ordering::Relaxed));

        // Need to create a new fork.
        let task_id =
            resolved_id.unwrap_or_else(|| ForkId::new(&fork.url, fork.evm_opts.fork_block_number));
        let needs_bal = prewarm_bal && !already_prewarmed;
        let future = Box::pin(create_fork(fork, expected_identity, needs_bal, exact_block));
        self.pending_tasks.push(ForkTask::Create {
            future,
            id: task_id,
            prewarm_bal,
            no_fork_bal,
            sender,
            additional_senders: Vec::new(),
        });
    }

    fn insert_new_fork(
        &mut self,
        fork_id: ForkId,
        fork: CreatedFork<N, SPEC, BLOCK>,
        sender: CreateSender<N, SPEC, BLOCK>,
        additional_senders: Vec<CreateSender<N, SPEC, BLOCK>>,
    ) {
        self.forks.insert(fork_id.clone(), fork.clone());
        let resolved = fork.fork.clone();
        let _ = sender.send(Ok(ForkResult {
            id: fork_id.clone(),
            backend: fork.backend.clone(),
            env: fork.evm_env.clone(),
            fork: resolved.clone(),
        }));

        // Notify all additional senders and track unique forkIds.
        for sender in additional_senders {
            let next_fork_id = fork.inc_senders(fork_id.clone());
            self.forks.insert(next_fork_id.clone(), fork.clone());
            let _ = sender.send(Ok(ForkResult {
                id: next_fork_id,
                backend: fork.backend.clone(),
                env: fork.evm_env.clone(),
                fork: resolved.clone(),
            }));
        }
    }

    fn on_request(&mut self, req: Request<N, SPEC, BLOCK>) {
        match req {
            Request::CloneForks(sender) => {
                let _ = sender.send(self.forks.clone());
            }
            Request::CreateFork(fork, sender) => self.create_fork(*fork, sender),
            Request::GetFork(fork_id, sender) => {
                let fork = self.forks.get(&fork_id).map(|f| f.backend.clone());
                let _ = sender.send(fork);
            }
            Request::RollFork(fork_id, block, sender) => {
                if let Some(fork) = self.forks.get(&fork_id) {
                    trace!(target: "fork::multi", "rolling {} to {}", fork_id, block);
                    let expected_identity = Some(fork.fork.context());
                    let mut opts = fork.opts.clone();
                    opts.evm_opts.fork_block_number = Some(block);
                    opts.evm_opts.fork_block_number_is_inferred = false;
                    self.create_fork_with_identity(opts, expected_identity, false, None, sender)
                } else {
                    let _ =
                        sender.send(Err(eyre::eyre!("No matching fork exists for {}", fork_id)));
                }
            }
            Request::RollForkExact(fork_id, block, prewarm_bal, sender) => {
                if let Some(fork) = self.forks.get(&fork_id) {
                    trace!(target: "fork::multi", "rolling {} to exact block {:?}", fork_id, block);
                    let mut opts = fork.opts.clone();
                    opts.evm_opts.fork_block_number = Some(block.number);
                    opts.evm_opts.fork_state_by_number = false;
                    opts.evm_opts.fork_block_number_is_inferred = false;
                    self.create_fork_with_identity(
                        opts,
                        None,
                        prewarm_bal,
                        Some((fork.fork.clone(), block)),
                        sender,
                    )
                } else {
                    let _ =
                        sender.send(Err(eyre::eyre!("No matching fork exists for {}", fork_id)));
                }
            }
            Request::GetEvmEnv(fork_id, sender) => {
                let _ = sender.send(self.forks.get(&fork_id).map(|fork| fork.evm_env.clone()));
            }
            Request::ShutDown(sender) => {
                trace!(target: "fork::multi", "received shutdown signal");
                // We're emptying all fork backends, this way we ensure all caches get flushed.
                self.forks.clear();
                self.handlers.clear();
                let _ = sender.send(());
            }
            Request::GetForkInfo(fork_id, sender) => {
                let _ = sender.send(self.forks.get(&fork_id).map(|fork| fork.fork.clone()));
            }
            Request::GetForkUrl(fork_id, sender) => {
                let fork = self.forks.get(&fork_id).map(|f| f.opts.url.clone());
                let _ = sender.send(fork);
            }
            Request::GetForkOptions(fork_id, sender) => {
                let fork = self.forks.get(&fork_id).map(|f| f.opts.clone());
                let _ = sender.send(fork);
            }
        }
    }
}

// Drives all handler to completion.
// This future will finish once all underlying BackendHandler are completed.
impl<
    N: Network,
    SPEC: Into<SpecId> + Default + Copy + Unpin + Send + 'static,
    BLOCK: FoundryBlock + ForkBlockEnv + Default + Unpin,
> Future for MultiForkHandler<N, SPEC, BLOCK>
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Receive new requests.
        loop {
            match this.incoming.poll_next_unpin(cx) {
                Poll::Ready(Some(req)) => this.on_request(req),
                Poll::Ready(None) => {
                    // Channel closed, but we still need to drive the fork handlers to completion.
                    trace!(target: "fork::multi", "request channel closed");
                    break;
                }
                Poll::Pending => break,
            }
        }

        // Advance all tasks.
        for n in (0..this.pending_tasks.len()).rev() {
            let task = this.pending_tasks.swap_remove(n);
            match task {
                ForkTask::Create {
                    mut future,
                    id,
                    prewarm_bal,
                    no_fork_bal,
                    sender,
                    additional_senders,
                } => {
                    if let Poll::Ready(resp) = future.poll_unpin(cx) {
                        let resp = resp.and_then(|(fork_id, opts, resolved, evm_env, mut bal)| {
                            let (fork_id, fork) = if let Some(mut cached) =
                                this.forks.get(&fork_id).cloned()
                            {
                                if bal.is_some()
                                    && resolved.fingerprint() != cached.fork.fingerprint()
                                {
                                    debug!(target: "backend::fork", "ignoring fork BAL for a different cache identity");
                                    bal = None;
                                }
                                // Share the remote cache, RPC client, and block while retaining
                                // this consumer's selector, execution environment, and BAL
                                // policy. The fork ID includes the block hash, so the cached
                                // block is the same block.
                                cached.opts = opts;
                                cached.fork = Fork {
                                    client: cached.fork.client,
                                    block: cached.fork.block,
                                    ..resolved
                                };
                                cached.evm_env = evm_env;
                                (cached.inc_senders(fork_id), cached)
                            } else {
                                // Only a new backend loads the disk cache.
                                let (backend, handler) =
                                    create_backend(&opts, &resolved, &evm_env.block_env)?;
                                this.handlers.push((fork_id.clone(), handler));
                                (fork_id, CreatedFork::new(opts, resolved, evm_env, backend))
                            };
                            Ok((fork_id, fork, bal))
                        });
                        match resp {
                            Ok((fork_id, fork, bal)) => {
                                // Apply only after choosing the backend, including an existing
                                // cache.
                                if let Some(bal) = bal {
                                    bal::cache_bal(&fork.backend.data(), bal);
                                    fork.bal_prewarmed.store(true, Ordering::Relaxed);
                                }
                                this.insert_new_fork(fork_id, fork, sender, additional_senders);
                            }
                            Err(err) => {
                                let _ = sender.send(Err(eyre::eyre!("{err}")));
                                for sender in additional_senders {
                                    let _ = sender.send(Err(eyre::eyre!("{err}")));
                                }
                            }
                        }
                    } else {
                        this.pending_tasks.push(ForkTask::Create {
                            future,
                            id,
                            prewarm_bal,
                            no_fork_bal,
                            sender,
                            additional_senders,
                        });
                    }
                }
            }
        }

        // Advance all handlers.
        for n in (0..this.handlers.len()).rev() {
            let (id, mut handler) = this.handlers.swap_remove(n);
            match handler.poll_unpin(cx) {
                Poll::Ready(_) => {
                    trace!(target: "fork::multi", "fork {:?} completed", id);
                }
                Poll::Pending => {
                    this.handlers.push((id, handler));
                }
            }
        }

        if this.handlers.is_empty() && this.incoming.is_done() {
            trace!(target: "fork::multi", "completed");
            return Poll::Ready(());
        }

        // Periodically flush cached RPC state.
        if this
            .flush_cache_interval
            .as_mut()
            .map(|interval| interval.poll_tick(cx).is_ready())
            .unwrap_or_default()
            && !this.forks.is_empty()
        {
            trace!(target: "fork::multi", "tick flushing caches");
            // Only the registry owning a handler flushes its cache, once per backend.
            let forks = this
                .handlers
                .iter()
                .filter_map(|(id, _)| this.forks.get(id))
                .map(|fork| fork.backend.clone())
                .collect::<Vec<_>>();
            // Flush this on new thread to not block here.
            std::thread::Builder::new()
                .name("flusher".into())
                .spawn(move || {
                    for fork in forks {
                        fork.flush_cache();
                    }
                })
                .expect("failed to spawn thread");
        }

        Poll::Pending
    }
}

/// Tracks the created Fork
#[derive(Debug, Clone)]
struct CreatedFork<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    /// How the fork was initially created.
    opts: CreateFork,
    /// The immutable remote fork owned by this backend.
    fork: Fork,
    /// The resolved EVM environment (fetched from the provider).
    evm_env: EvmEnv<SPEC, BLOCK>,
    /// Copy of the sender.
    backend: SharedBackend<N, BLOCK>,
    /// How many consumers there are, since a `SharedBacked` can be used by multiple
    /// consumers.
    num_senders: Arc<AtomicUsize>,
    /// Whether this exact shared backend has successfully received a validated parent BAL.
    bal_prewarmed: Arc<AtomicBool>,
}

impl<N: Network, SPEC, BLOCK: ForkBlockEnv> CreatedFork<N, SPEC, BLOCK> {
    pub fn new(
        opts: CreateFork,
        fork: Fork,
        evm_env: EvmEnv<SPEC, BLOCK>,
        backend: SharedBackend<N, BLOCK>,
    ) -> Self {
        Self {
            opts,
            fork,
            evm_env,
            backend,
            num_senders: Arc::new(AtomicUsize::new(1)),
            bal_prewarmed: Arc::default(),
        }
    }

    /// Increment senders and return unique identifier of the fork.
    fn inc_senders(&self, fork_id: ForkId) -> ForkId {
        format!(
            "{}-{}",
            fork_id.as_str(),
            self.num_senders.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
        .into()
    }
}

/// A type that's used to signaling the `MultiForkHandler` when it's time to shut down.
///
/// This is essentially a sync on drop, so that the `MultiForkHandler` can flush all rpc cashes.
///
/// This type intentionally does not implement `Clone` since it's intended that there's only once
/// instance.
#[derive(Debug)]
struct ShutDownMultiFork<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    handler: Option<Sender<Request<N, SPEC, BLOCK>>>,
    // Keep shared backend handlers alive until this registry has shut down.
    _parent: Option<Arc<Self>>,
}

impl<N: Network, SPEC, BLOCK: ForkBlockEnv> Drop for ShutDownMultiFork<N, SPEC, BLOCK> {
    fn drop(&mut self) {
        trace!(target: "fork::multi", "initiating shutdown");
        let (sender, rx) = oneshot_channel();
        let req = Request::ShutDown(sender);
        if let Some(mut handler) = self.handler.take()
            && handler.try_send(req).is_ok()
        {
            let _ = rx.recv();
            trace!(target: "fork::cache", "multifork backend shutdown");
        }
    }
}

/// Creates a new fork.
///
/// This resolves the fork block and environment through the endpoint. The handler creates the
/// backend with [`create_backend`] only if no backend exists for the fork ID.
async fn create_fork<
    SPEC: Into<SpecId> + Default + Copy + Send,
    BLOCK: FoundryBlock + ForkBlockEnv + Default,
>(
    mut fork: CreateFork,
    expected_identity: Option<ForkContext>,
    prewarm_bal: bool,
    exact_block: Option<(Fork, BlockNumHash)>,
) -> eyre::Result<(ForkId, CreateFork, Fork, EvmEnv<SPEC, BLOCK>, Option<BlockAccessList>)> {
    // Ensure evm_opts reflects the fork URL (may differ from the resolved CreateFork url when
    // created via cheatcodes, where evm_opts is cloned from the base config).
    let execution_networks = fork.evm_opts.networks;
    let require_endpoint_family_match =
        fork.evm_opts.fork_network_is_inferred || !execution_networks.has_network_selection();
    let targets_new_endpoint =
        fork.evm_opts.fork_url.as_ref().is_some_and(|endpoint| endpoint != &fork.url)
            || fork
                .evm_opts
                .fork_endpoint
                .as_ref()
                .is_some_and(|identity| identity.endpoint != fork.url);
    if targets_new_endpoint {
        // The EVM implementation is already fixed, so use its family as the fallback for a custom
        // endpoint without metadata. Clear identity and chain values inferred from the old URL;
        // authoritative metadata from the new endpoint is still checked below.
        fork.evm_opts.fork_endpoint = None;
        fork.evm_opts.expected_fork_endpoint = None;
        fork.evm_opts.fork_network_is_inferred = false;
        if fork.evm_opts.fork_chain_id_is_inferred {
            fork.evm_opts.env.chain_id = None;
            fork.evm_opts.fork_chain_id_is_inferred = false;
        }
        if fork.evm_opts.fork_block_number_is_inferred {
            fork.evm_opts.fork_block_number = None;
            fork.evm_opts.fork_block_number_is_inferred = false;
        }
    }
    fork.evm_opts.fork_url = Some(fork.url.clone());

    // Initialise the fork environment.
    // Here we use [`AnyNetwork`] to maximize compatibility with custom chains, aligned with
    // `EvmOpts::env` impl.
    let resolved = if let Some((parent, block)) = exact_block {
        let provider = parent.provider::<AnyNetwork>();
        fork.evm_opts.check_fork_endpoint(&provider, &parent).await?;
        let fork_state = parent.at_block(block).await?;
        fork.evm_opts.check_fork_endpoint(&provider, &fork_state).await?;
        fork_state
    } else {
        fork.evm_opts.prepare_fork().await?.expect("fork URL is configured")
    };
    let any_provider = resolved.provider::<AnyNetwork>();
    let fork_context = resolved.context();
    let evm_env = fork.evm_opts.fork_env_from_block::<SPEC, BLOCK, AnyNetwork>(
        fork.evm_opts.chain_id_override().unwrap_or(fork_context.execution_chain_id),
        fork_context.source_chain_id,
        &resolved.block,
    );
    if require_endpoint_family_match
        && !execution_networks.supports_fork_source(&fork_context.network_profile)
    {
        eyre::bail!(
            "cannot create a `{}` fork with an EVM instantiated for `{}`; run the script with --rpc-url pointing to the forked chain to select its EVM",
            fork_context.network,
            execution_networks.execution_network()
        );
    }
    if let Some(expected) = expected_identity {
        eyre::ensure!(
            fork_context.has_same_endpoint_identity(expected),
            "fork endpoint identity changed while the fork was being rolled"
        );
    }
    let bal = if prewarm_bal {
        bal::prepare(&any_provider, &resolved, &resolved.block).await
    } else {
        None
    };
    let fork_id = ForkId::resolved(&fork.url, &resolved);

    Ok((fork_id, fork, resolved, evm_env, bal))
}

/// Creates the backend of a resolved fork.
///
/// This loads the disk cache of the fork block, so it only runs for a new fork ID.
fn create_backend<N: Network, BLOCK: FoundryBlock + ForkBlockEnv>(
    fork: &CreateFork,
    resolved: &Fork,
    block_env: &BLOCK,
) -> eyre::Result<(SharedBackend<N, BLOCK>, BackendHandler<N, BLOCK>)> {
    let fork_context = resolved.context();
    let number = resolved.number();
    let account_fetch_policy = crate::backend::account_fetch_policy_for_source(
        fork_context.source_chain_id,
        fork_context.network_profile,
    );
    let meta = BlockchainDbMeta::new(block_env.clone(), fork.url.clone())
        .with_fork_identity(resolved.hash(), resolved.source_id())
        .with_account_fetch_policy(account_fetch_policy);

    // Determine the cache path if caching is enabled.
    // Number-addressed state can follow a replacement block, so keep it in memory only.
    let cache_path = if fork.enable_caching && !resolved.state_by_number {
        Config::foundry_block_cache_dir(fork_context.source_chain_id, number)
    } else {
        None
    };

    let provider = resolved.provider::<N>();
    let db = BlockchainDb::new(meta, cache_path);
    let anchor = ForkBlock::with_rpc_number(
        block_env.number().saturating_to(),
        resolved.number(),
        resolved.hash(),
    );
    if resolved.state_by_number {
        SharedBackend::new_with_anchor_by_number(provider, db, anchor)
    } else {
        SharedBackend::new_with_anchor(provider, db, anchor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opts::EvmOpts;
    use alloy_chains::NamedChain;
    use alloy_eips::eip7928::{AccountChanges, BlockAccessIndex, SlotChanges, StorageChange};
    use alloy_network::TransactionBuilder;
    use alloy_primitives::{Address, B256, U256, bytes};
    use alloy_provider::{Provider, ProviderBuilder, mock::Asserter};
    use alloy_rpc_types::TransactionRequest;
    use alloy_serde::WithOtherFields;
    use foundry_evm_networks::{NetworkConfigs, NetworkVariant};
    use foundry_fork_db::AccountFetchPolicy;
    use foundry_test_utils::rpc::{
        spawn_rpc_proxy_method_not_found_before, spawn_rpc_proxy_recording_method,
    };
    use futures::{channel::oneshot, task::noop_waker_ref};
    use revm::context::{BlockEnv, TxEnv};
    use std::sync::mpsc::{Receiver as OneshotReceiver, TryRecvError};

    fn context(block_number: u64) -> ForkContext {
        ForkContext {
            execution_chain_id: 1,
            source_chain_id: 1,
            network: NetworkVariant::Ethereum,
            network_profile: NetworkConfigs::default(),
            block_number,
            hardfork: None,
            instance_id: None,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        }
    }

    #[test]
    fn resolved_fork_ids_include_hash_and_source_identity() {
        let url = "http://localhost:8545";
        let first = Fork::test(
            url,
            None,
            None,
            Some(1),
            BlockNumHash::new(1, B256::with_last_byte(1)),
            context(1),
        );
        let replacement = Fork::test(
            url,
            None,
            None,
            Some(1),
            BlockNumHash::new(1, B256::with_last_byte(2)),
            context(1),
        );
        let authenticated = Fork::test(
            url,
            Some(&["Authorization: secret".to_string()]),
            None,
            Some(1),
            BlockNumHash::new(1, B256::with_last_byte(1)),
            context(1),
        );

        assert_ne!(ForkId::resolved(url, &first), ForkId::resolved(url, &replacement));
        assert_ne!(ForkId::resolved(url, &first), ForkId::resolved(url, &authenticated));
        let mut numbered = first.clone();
        numbered.state_by_number = true;
        assert_ne!(ForkId::resolved(url, &first), ForkId::resolved(url, &numbered));
        assert_eq!(ForkId::resolved(url, &first), ForkId::exact(url, &numbered, first.block()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_state_by_number_does_not_load_or_overwrite_hash_cache() {
        let chain_id = u64::from_be_bytes(B256::random()[..8].try_into().unwrap());
        let (_api, handle) = anvil::spawn(
            anvil::NodeConfig::test().with_chain_id(Some(chain_id)).with_no_mining(true),
        )
        .await;
        let opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            fork_block_number: Some(0),
            ..Default::default()
        };
        let resolved = opts.prepare_fork().await.unwrap().unwrap();
        let (env, _) = opts.env_at_fork::<SpecId, BlockEnv, TxEnv>(Some(&resolved)).await.unwrap();
        let path = Config::foundry_block_cache_dir(chain_id, resolved.number()).unwrap();
        assert!(!path.exists());
        let meta = BlockchainDbMeta::new(env.block_env, handle.http_endpoint())
            .with_fork_identity(resolved.hash(), resolved.source_id());
        let cached = BlockchainDb::new(meta.clone(), Some(path.clone()));
        let address = Address::with_last_byte(0x42);
        cached.storage().write().entry(address).or_default().insert(U256::ZERO, U256::from(42));
        cached.cache().flush();
        assert!(path.exists());

        for state_by_number in [true, false] {
            let fork = CreateFork {
                url: handle.http_endpoint(),
                enable_caching: true,
                evm_opts: EvmOpts { fork_state_by_number: state_by_number, ..opts.clone() },
            };
            let (_, fork, fork_state, evm_env, _) =
                create_fork::<SpecId, BlockEnv>(fork, None, false, None).await.unwrap();
            let (backend, handler) =
                create_backend::<AnyNetwork, _>(&fork, &fork_state, &evm_env.block_env).unwrap();
            assert_eq!(backend.data().storage.read().contains_key(&address), !state_by_number);
            if state_by_number {
                backend
                    .data()
                    .storage
                    .write()
                    .entry(address)
                    .or_default()
                    .insert(U256::ZERO, U256::from(99));
            }
            backend.flush_cache();
            drop(backend);
            drop(handler);
            let reloaded = BlockchainDb::new(meta.clone(), Some(path.clone()));
            assert_eq!(reloaded.storage().read()[&address][&U256::ZERO], U256::from(42));
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn account_fetch_policy_follows_source_identity() {
        assert_eq!(
            crate::backend::account_fetch_policy_for_source(
                NamedChain::Tempo as u64,
                NetworkConfigs::with_ethereum(),
            ),
            AccountFetchPolicy::RequireAccountInfo,
        );
        assert_eq!(
            crate::backend::account_fetch_policy_for_source(
                NamedChain::Mainnet as u64,
                NetworkConfigs::with_tempo(),
            ),
            AccountFetchPolicy::Auto,
        );
        assert_eq!(
            crate::backend::account_fetch_policy_for_source(123_456, NetworkConfigs::with_tempo(),),
            AccountFetchPolicy::RequireAccountInfo,
        );
    }

    #[test]
    fn fork_bal_pending_requests_preserve_opt_out() {
        let url = "http://localhost:8545";
        let resolved = Fork::test(
            url,
            None,
            None,
            Some(1),
            BlockNumHash::new(1, B256::with_last_byte(1)),
            context(1),
        );
        let (_, receiver) = channel(1);
        let mut handler =
            MultiForkHandler::<AnyNetwork, SpecId, revm::context::BlockEnv>::new(receiver);
        let mut fork = CreateFork {
            enable_caching: false,
            url: url.to_string(),
            evm_opts: Default::default(),
        };
        let (sender, _) = oneshot_channel();
        handler.create_fork_with_identity(
            fork.clone(),
            None,
            false,
            Some((resolved.clone(), resolved.block())),
            sender,
        );
        fork.evm_opts.no_fork_bal = true;
        let (sender, _) = oneshot_channel();
        handler.create_fork_with_identity(
            fork.clone(),
            None,
            false,
            Some((resolved.clone(), resolved.block())),
            sender,
        );
        assert_eq!(handler.pending_tasks.len(), 2, "each fork must retain its opt-out policy");
        let (sender, _) = oneshot_channel();
        handler.create_fork_with_identity(
            fork.clone(),
            None,
            false,
            Some((resolved.clone(), resolved.block())),
            sender,
        );
        assert_eq!(handler.pending_tasks.len(), 2, "identical policies can share creation");
        fork.evm_opts.no_fork_bal = false;
        let (sender, _) = oneshot_channel();
        handler.create_fork_with_identity(
            fork.clone(),
            None,
            true,
            Some((resolved.clone(), resolved.block())),
            sender,
        );
        assert_eq!(handler.pending_tasks.len(), 3, "prewarming must not join ordinary creation");
        let (sender, _) = oneshot_channel();
        handler.create_fork_with_identity(
            fork,
            None,
            true,
            Some((resolved.clone(), resolved.block())),
            sender,
        );
        assert_eq!(handler.pending_tasks.len(), 3, "matching prewarm requests can share creation");
    }

    #[test]
    fn fork_bal_fills_selected_cache_for_equivalent_profiles() {
        let url = "http://localhost:8545";
        let block = BlockNumHash::new(1, B256::with_last_byte(1));
        let address = Address::with_last_byte(1);
        let create = |resolved: Fork| {
            let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
                .connect_mocked_client(Asserter::new());
            let env = EvmEnv::<SpecId>::default();
            let meta = BlockchainDbMeta::new(env.block_env.clone(), url.to_string())
                .with_fork_identity(resolved.hash(), resolved.source_id());
            let (backend, handler) = SharedBackend::new_with_anchor(
                provider,
                BlockchainDb::new(meta, None),
                ForkBlock::with_rpc_number(1, 1, resolved.hash()),
            )
            .unwrap();
            let opts = CreateFork {
                enable_caching: false,
                url: url.to_string(),
                evm_opts: Default::default(),
            };
            (CreatedFork::new(opts, resolved, env, backend), handler)
        };

        for profile in [NetworkConfigs::default(), NetworkConfigs::with_ethereum()] {
            let cached = Fork::test(url, None, None, None, block, context(1));
            let candidate = Fork::test(
                url,
                None,
                None,
                Some(1),
                block,
                ForkContext { network_profile: profile, ..context(1) },
            );
            let id = ForkId::resolved(url, &candidate);
            assert_eq!(id, ForkId::resolved(url, &cached));
            assert_eq!(candidate.fingerprint(), cached.fingerprint());
            let (cached, cached_handler) = create(cached);
            let cached_db = cached.backend.data();
            let bal = vec![AccountChanges::new(address).with_storage_change(SlotChanges::new(
                U256::ONE,
                vec![StorageChange::new(BlockAccessIndex::new(1), U256::from(42))],
            ))];
            let future = futures::future::ready(Ok((
                id.clone(),
                cached.opts.clone(),
                candidate,
                cached.evm_env.clone(),
                Some(bal),
            )));
            let (_incoming, receiver) = channel(1);
            let mut manager = MultiForkHandler::<AnyNetwork, SpecId, BlockEnv>::new(receiver);
            manager.forks.insert(id.clone(), cached);
            manager.handlers.push((id.clone(), cached_handler));
            let (sender, receiver) = oneshot_channel();
            manager.pending_tasks.push(ForkTask::Create {
                future: Box::pin(future),
                id: id.clone(),
                prewarm_bal: true,
                no_fork_bal: false,
                sender,
                additional_senders: Vec::new(),
            });

            assert!(manager.poll_unpin(&mut Context::from_waker(noop_waker_ref())).is_pending());
            let result = receiver.try_recv().unwrap().unwrap();
            assert!(Arc::ptr_eq(&result.backend.data(), &cached_db));
            // Reusing the cached backend creates no second backend.
            assert_eq!(manager.handlers.len(), 1);
            assert_eq!(
                cached_db
                    .storage
                    .read()
                    .get(&address)
                    .and_then(|slots| slots.get(&U256::ONE))
                    .copied(),
                Some(U256::from(42)),
            );
            assert!(manager.forks[&id].bal_prewarmed.load(Ordering::Relaxed));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_bal_reuses_prewarmed_cache_only_for_equivalent_identities() {
        let (api, handle) = anvil::spawn(
            anvil::NodeConfig::test()
                .with_chain_id(Some(1u64))
                .with_hardfork(Some(anvil::EthereumHardfork::Amsterdam.into()))
                .with_genesis_timestamp(Some(1_800_000_000u64))
                .with_no_mining(true),
        )
        .await;
        let address = Address::with_last_byte(0x42);
        api.anvil_set_code(address, bytes!("602a60015500")).await.unwrap();
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(handle.dev_accounts().next().unwrap())
                .with_to(address)
                .with_nonce(0)
                // Enough for the EIP-8037 state gas of creating slot one.
                .with_gas_limit(1_000_000)
                .with_gas_price(2_000_000_000),
        ))
        .await
        .unwrap();
        api.mine_one().await.unwrap();
        let block = handle.http_provider().get_block_by_number(1.into()).await.unwrap().unwrap();
        let block = BlockNumHash::new(1, block.header.hash);
        // Expose native BALs through an endpoint without mutable Anvil identity.
        let endpoint = spawn_rpc_proxy_method_not_found_before(
            handle.http_endpoint(),
            "anvil_nodeInfo",
            usize::MAX,
        )
        .await;
        let (endpoint, bal_requests) =
            spawn_rpc_proxy_recording_method(endpoint, "eth_getBlockAccessList").await;
        let (endpoint, probes) = spawn_rpc_proxy_recording_method(endpoint, "anvil_nodeInfo").await;
        let (_incoming, receiver) = channel(1);
        let mut manager = MultiForkHandler::<AnyNetwork, SpecId, BlockEnv>::new(receiver);
        let mut first = None;
        let mut probe_counts = Vec::new();
        let other_headers = ["X-Test-Source: other".to_string()];

        for (profile, headers) in [
            (NetworkConfigs::default(), None),
            (NetworkConfigs::with_ethereum(), None),
            (NetworkConfigs::default(), None),
            (NetworkConfigs::default(), Some(other_headers.as_slice())),
        ] {
            let evm_opts = EvmOpts {
                fork_url: Some(endpoint.clone()),
                fork_block_number: Some(1),
                fork_headers: headers.map(<[_]>::to_vec),
                networks: profile,
                ..Default::default()
            };
            let resolved = evm_opts.prepare_fork().await.unwrap().unwrap();
            assert_eq!(resolved.block(), block);
            let fingerprint = resolved.fingerprint();
            let fork = CreateFork { enable_caching: false, url: endpoint.clone(), evm_opts };
            let before = probes.lock().unwrap().len();
            let (sender, receiver) = oneshot_channel();
            manager.create_fork_with_identity(
                fork,
                None,
                true,
                Some((resolved.clone(), resolved.block())),
                sender,
            );
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                futures::future::poll_fn(|cx| {
                    assert!(manager.poll_unpin(cx).is_pending());
                    match receiver.try_recv() {
                        Ok(result) => Poll::Ready(result),
                        Err(std::sync::mpsc::TryRecvError::Empty) => Poll::Pending,
                        Err(error) => panic!("fork response channel closed: {error}"),
                    }
                }),
            )
            .await
            .unwrap()
            .unwrap();

            let db = result.backend.data();
            assert_eq!(
                Arc::ptr_eq(first.get_or_insert_with(|| db.clone()), &db),
                headers.is_none(),
            );
            assert_eq!(result.fork.fingerprint(), fingerprint);
            assert_eq!(db.storage.read()[&address][&U256::ONE], U256::from(42));
            assert!(manager.forks[&result.id].bal_prewarmed.load(Ordering::Relaxed));
            assert_eq!(bal_requests.lock().unwrap().len(), if headers.is_some() { 2 } else { 1 });
            probe_counts.push(probes.lock().unwrap().len() - before);
        }

        // Only cold caches need the two source probes surrounding BAL preparation.
        assert_eq!(probe_counts[0], probe_counts[1] + 2);
        assert_eq!(probe_counts[1], probe_counts[2]);
        assert_eq!(probe_counts[0], probe_counts[3]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_bal_shared_backend_preserves_opposite_policies_on_roll() {
        let (api, handle) = anvil::spawn(
            anvil::NodeConfig::test()
                .with_chain_id(Some(1u64))
                .with_hardfork(Some(anvil::EthereumHardfork::Amsterdam.into()))
                .with_genesis_timestamp(Some(1_800_000_000u64))
                .with_no_mining(true),
        )
        .await;
        let mut blocks = Vec::new();
        for number in 1..=3 {
            api.mine_one().await.unwrap();
            let block =
                handle.http_provider().get_block_by_number(number.into()).await.unwrap().unwrap();
            blocks.push(BlockNumHash::new(number, block.header.hash));
        }
        // Expose native BALs through an endpoint without mutable Anvil identity.
        let endpoint = spawn_rpc_proxy_method_not_found_before(
            handle.http_endpoint(),
            "anvil_nodeInfo",
            usize::MAX,
        )
        .await;
        let (endpoint, bal_requests) =
            spawn_rpc_proxy_recording_method(endpoint, "eth_getBlockAccessList").await;
        let resolved = EvmOpts {
            fork_url: Some(endpoint.clone()),
            fork_block_number: Some(1),
            ..Default::default()
        }
        .prepare_fork()
        .await
        .unwrap()
        .unwrap();
        assert_eq!(resolved.block(), blocks[0]);

        for disabled_first in [false, true] {
            let (_incoming, receiver) = channel(1);
            let mut manager = MultiForkHandler::<AnyNetwork, SpecId, BlockEnv>::new(receiver);
            bal_requests.lock().unwrap().clear();
            let mut requests = [false, true].map(|no_fork_bal| {
                let fork = CreateFork {
                    enable_caching: false,
                    url: endpoint.clone(),
                    evm_opts: EvmOpts {
                        fork_url: Some(endpoint.clone()),
                        fork_block_number: Some(1),
                        no_fork_bal,
                        ..Default::default()
                    },
                };
                let (sender, receiver) = oneshot_channel();
                manager.create_fork(fork, sender);
                let ForkTask::Create { future, .. } = manager.pending_tasks.last_mut().unwrap();
                let original = std::mem::replace(future, Box::pin(futures::future::pending()));
                let (release, gate) = oneshot::channel();
                // Run real creation, but explicitly control which result reaches backend reuse.
                *future = Box::pin(async move {
                    let result = original.await;
                    gate.await.unwrap();
                    result
                });
                (no_fork_bal, release, receiver)
            });
            assert_eq!(manager.pending_tasks.len(), 2);
            if disabled_first {
                requests.reverse();
            }
            let [
                (first_policy, first_gate, first_receiver),
                (second_policy, second_gate, second_receiver),
            ] = requests;
            first_gate.send(()).unwrap();
            let first = complete_fork(&mut manager, first_receiver).await;
            assert!(matches!(second_receiver.try_recv(), Err(TryRecvError::Empty)));
            assert_eq!(manager.pending_tasks.len(), 1);
            second_gate.send(()).unwrap();
            let second = complete_fork(&mut manager, second_receiver).await;

            assert_ne!(first.id, second.id);
            assert!(Arc::ptr_eq(&first.backend.data(), &second.backend.data()));
            assert!(Arc::ptr_eq(first.fork.client.inner(), second.fork.client.inner()));
            assert!(Arc::ptr_eq(&first.fork.block, &second.fork.block));
            assert_eq!(manager.forks[&first.id].opts.evm_opts.no_fork_bal, first_policy);
            assert_eq!(manager.forks[&second.id].opts.evm_opts.no_fork_bal, second_policy);
            assert!(bal_requests.lock().unwrap().is_empty());

            // Each roll targets a cold block so shared prewarming cannot mask a lost policy.
            for (no_fork_bal, fork, block) in
                [(first_policy, first, blocks[1]), (second_policy, second, blocks[2])]
            {
                bal_requests.lock().unwrap().clear();
                let (sender, receiver) = oneshot_channel();
                manager.on_request(Request::RollForkExact(fork.id, block, true, sender));
                let rolled = complete_fork(&mut manager, receiver).await;
                assert_eq!(rolled.fork.block(), block);
                assert_eq!(manager.forks[&rolled.id].opts.evm_opts.no_fork_bal, no_fork_bal);
                assert_eq!(
                    manager.forks[&rolled.id].bal_prewarmed.load(Ordering::Relaxed),
                    !no_fork_bal,
                );
                let expected =
                    if no_fork_bal { vec![] } else { vec![serde_json::json!([block.hash])] };
                assert_eq!(*bal_requests.lock().unwrap(), expected);
            }
        }
    }

    async fn complete_fork(
        manager: &mut MultiForkHandler<AnyNetwork, SpecId, BlockEnv>,
        receiver: OneshotReceiver<eyre::Result<ForkResult<AnyNetwork, SpecId, BlockEnv>>>,
    ) -> ForkResult<AnyNetwork, SpecId, BlockEnv> {
        tokio::time::timeout(
            Duration::from_secs(10),
            futures::future::poll_fn(|cx| {
                assert!(manager.poll_unpin(cx).is_pending());
                match receiver.try_recv() {
                    Ok(result) => Poll::Ready(result),
                    Err(TryRecvError::Empty) => Poll::Pending,
                    Err(error) => panic!("fork response channel closed: {error}"),
                }
            }),
        )
        .await
        .unwrap()
        .unwrap()
    }
}
