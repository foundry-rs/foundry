//! Support for running multiple fork backends.
//!
//! The design is similar to the single `SharedBackend`, `BackendHandler` but supports multiple
//! concurrently active pairs at once.

use super::{CreateFork, ResolvedFork};
use crate::{FoundryBlock, opts::ForkContext};
use alloy_eips::BlockNumHash;
use alloy_evm::EvmEnv;
use alloy_network::{AnyNetwork, Network};
use alloy_primitives::{U256, map::HashMap};
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
        atomic::AtomicUsize,
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
    fn resolved(url: &str, fork: &ResolvedFork) -> Self {
        let mut id = Self::new_with_context(url, Some(fork.number()), Some(&fork.context())).0;
        write!(id, "#{}:{}", fork.hash(), fork.source_id()).unwrap();
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
    pub resolved: ResolvedFork,
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
        trace!(target: "fork::multi", "spawning multifork");

        let (fork, mut handler) = Self::new();

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
        let _shutdown = Arc::new(ShutDownMultiFork { handler: Some(handler.clone()) });
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
        trace!(?fork, ?block, "rolling fork to exact block");
        let (sender, rx) = oneshot_channel();
        let req = Request::RollForkExact(fork, block, sender);
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

    /// Updates block number and timestamp of given fork with new values.
    pub fn update_block(&self, fork: ForkId, number: U256, timestamp: U256) -> eyre::Result<()> {
        trace!(?fork, ?number, ?timestamp, "update fork block");
        self.handler
            .clone()
            .try_send(Request::UpdateBlock(fork, number, timestamp))
            .map_err(|e| eyre::eyre!("{:?}", e))
    }

    /// Updates the fork's entire env
    ///
    /// This is required for tx level forking where we need to fork off the `block - 1` state but
    /// still need use env settings for `env`.
    pub fn update_block_env(&self, fork: ForkId, env: BLOCK) -> eyre::Result<()>
    where
        BLOCK: fmt::Debug,
    {
        trace!(?fork, ?env, "update fork block");
        self.handler
            .clone()
            .try_send(Request::UpdateEnv(fork, env))
            .map_err(|e| eyre::eyre!("{:?}", e))
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

    /// Returns the corresponding fork url if it exists.
    ///
    /// Returns `None` if no matching fork is available.
    pub fn get_fork_url(&self, id: impl Into<ForkId>) -> eyre::Result<Option<String>> {
        let (sender, rx) = oneshot_channel();
        let req = Request::GetForkUrl(id.into(), sender);
        self.handler.clone().try_send(req).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()?)
    }
}

type CreateFuture<N, SPEC, BLOCK> = Pin<
    Box<
        dyn Future<
                Output = eyre::Result<(
                    ForkId,
                    CreatedFork<N, SPEC, BLOCK>,
                    BackendHandler<N, BLOCK>,
                )>,
            > + Send,
    >,
>;
type CreateSender<N, SPEC, BLOCK> = OneshotSender<eyre::Result<ForkResult<N, SPEC, BLOCK>>>;
type GetEvmEnvSender<SPEC, BLOCK> = OneshotSender<Option<EvmEnv<SPEC, BLOCK>>>;

/// Request that's send to the handler.
#[derive(Debug)]
enum Request<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    /// Creates a new ForkBackend.
    CreateFork(Box<CreateFork>, CreateSender<N, SPEC, BLOCK>),
    /// Returns the Fork backend for the `ForkId` if it exists.
    GetFork(ForkId, OneshotSender<Option<SharedBackend<N, BLOCK>>>),
    /// Adjusts the block that's being forked, by creating a new fork at the new block.
    RollFork(ForkId, u64, CreateSender<N, SPEC, BLOCK>),
    /// Adjusts the fork to an already resolved exact block.
    RollForkExact(ForkId, BlockNumHash, CreateSender<N, SPEC, BLOCK>),
    /// Returns the environment of the fork.
    GetEvmEnv(ForkId, GetEvmEnvSender<SPEC, BLOCK>),
    /// Updates the block number and timestamp of the fork.
    UpdateBlock(ForkId, U256, U256),
    /// Updates the block the entire block env,
    UpdateEnv(ForkId, BLOCK),
    /// Shutdowns the entire `MultiForkHandler`, see `ShutDownMultiFork`
    ShutDown(OneshotSender<()>),
    /// Returns the Fork Url for the `ForkId` if it exists.
    GetForkUrl(ForkId, OneshotSender<Option<String>>),
}

enum ForkTask<N: Network, SPEC, BLOCK: ForkBlockEnv> {
    /// Contains the future that will establish a new fork.
    Create(
        CreateFuture<N, SPEC, BLOCK>,
        ForkId,
        CreateSender<N, SPEC, BLOCK>,
        Vec<CreateSender<N, SPEC, BLOCK>>,
    ),
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
    SPEC: Into<SpecId> + Default + Copy + 'static,
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
    ) -> Option<&mut Vec<CreateSender<N, SPEC, BLOCK>>> {
        for ForkTask::Create(_, in_progress, _, additional) in &mut self.pending_tasks {
            if in_progress == id {
                return Some(additional);
            }
        }
        None
    }

    fn create_fork(&mut self, fork: CreateFork, sender: CreateSender<N, SPEC, BLOCK>) {
        self.create_fork_with_identity(fork, None, sender);
    }

    fn create_fork_with_identity(
        &mut self,
        fork: CreateFork,
        expected_identity: Option<ForkContext>,
        sender: CreateSender<N, SPEC, BLOCK>,
    ) {
        let resolved_id =
            fork.resolved.as_ref().map(|resolved| ForkId::resolved(&fork.url, resolved));
        trace!(?resolved_id, "creating fork");

        // Only deduplicate requests that already carry an exact identity. Unresolved requests at
        // the same URL and height can resolve to different blocks across a reorganization.
        if let Some(fork_id) = &resolved_id
            && let Some(in_progress) = self.find_in_progress_task(fork_id)
        {
            in_progress.push(sender);
            return;
        }

        // Need to create a new fork.
        let task_id =
            resolved_id.unwrap_or_else(|| ForkId::new(&fork.url, fork.evm_opts.fork_block_number));
        let task = Box::pin(create_fork(fork, expected_identity));
        self.pending_tasks.push(ForkTask::Create(task, task_id, sender, Vec::new()));
    }

    fn insert_new_fork(
        &mut self,
        fork_id: ForkId,
        fork: CreatedFork<N, SPEC, BLOCK>,
        sender: CreateSender<N, SPEC, BLOCK>,
        additional_senders: Vec<CreateSender<N, SPEC, BLOCK>>,
    ) {
        self.forks.insert(fork_id.clone(), fork.clone());
        let resolved = fork
            .opts
            .resolved
            .as_ref()
            .expect("created forks always retain their resolved identity")
            .clone();
        let _ = sender.send(Ok(ForkResult {
            id: fork_id.clone(),
            backend: fork.backend.clone(),
            env: fork.evm_env.clone(),
            resolved: resolved.clone(),
        }));

        // Notify all additional senders and track unique forkIds.
        for sender in additional_senders {
            let next_fork_id = fork.inc_senders(fork_id.clone());
            self.forks.insert(next_fork_id.clone(), fork.clone());
            let _ = sender.send(Ok(ForkResult {
                id: next_fork_id,
                backend: fork.backend.clone(),
                env: fork.evm_env.clone(),
                resolved: resolved.clone(),
            }));
        }
    }

    /// Update the fork's block entire env
    fn update_env(&mut self, fork_id: ForkId, env: BLOCK) {
        if let Some(fork) = self.forks.get_mut(&fork_id) {
            fork.evm_env.block_env = env;
        }
    }
    /// Update fork block number and timestamp. Used to preserve values set by `roll` and `warp`
    /// cheatcodes when new fork selected.
    fn update_block(&mut self, fork_id: ForkId, block_number: U256, block_timestamp: U256) {
        if let Some(fork) = self.forks.get_mut(&fork_id) {
            fork.evm_env.block_env.set_number(block_number);
            fork.evm_env.block_env.set_timestamp(block_timestamp);
        }
    }

    fn on_request(&mut self, req: Request<N, SPEC, BLOCK>) {
        match req {
            Request::CreateFork(fork, sender) => self.create_fork(*fork, sender),
            Request::GetFork(fork_id, sender) => {
                let fork = self.forks.get(&fork_id).map(|f| f.backend.clone());
                let _ = sender.send(fork);
            }
            Request::RollFork(fork_id, block, sender) => {
                if let Some(fork) = self.forks.get(&fork_id) {
                    trace!(target: "fork::multi", "rolling {} to {}", fork_id, block);
                    let expected_identity = fork.opts.resolved.as_ref().map(ResolvedFork::context);
                    let mut opts = fork.opts.clone();
                    opts.evm_opts.fork_block_number = Some(block);
                    opts.evm_opts.fork_block_number_is_inferred = false;
                    opts.resolved = None;
                    self.create_fork_with_identity(opts, expected_identity, sender)
                } else {
                    let _ =
                        sender.send(Err(eyre::eyre!("No matching fork exists for {}", fork_id)));
                }
            }
            Request::RollForkExact(fork_id, block, sender) => {
                if let Some(fork) = self.forks.get(&fork_id) {
                    trace!(target: "fork::multi", "rolling {} to exact block {:?}", fork_id, block);
                    let mut opts = fork.opts.clone();
                    opts.evm_opts.fork_block_number = Some(block.number);
                    opts.evm_opts.fork_block_number_is_inferred = false;
                    opts.resolved = Some(
                        opts.resolved
                            .as_ref()
                            .expect("an exact roll requires an existing resolved fork")
                            .at_block(block),
                    );
                    self.create_fork(opts, sender)
                } else {
                    let _ =
                        sender.send(Err(eyre::eyre!("No matching fork exists for {}", fork_id)));
                }
            }
            Request::GetEvmEnv(fork_id, sender) => {
                let _ = sender.send(self.forks.get(&fork_id).map(|fork| fork.evm_env.clone()));
            }
            Request::UpdateBlock(fork_id, block_number, block_timestamp) => {
                self.update_block(fork_id, block_number, block_timestamp);
            }
            Request::UpdateEnv(fork_id, block_env) => {
                self.update_env(fork_id, block_env);
            }
            Request::ShutDown(sender) => {
                trace!(target: "fork::multi", "received shutdown signal");
                // We're emptying all fork backends, this way we ensure all caches get flushed.
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

// Drives all handler to completion.
// This future will finish once all underlying BackendHandler are completed.
impl<
    N: Network,
    SPEC: Into<SpecId> + Default + Copy + Unpin + 'static,
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
                ForkTask::Create(mut fut, id, sender, additional_senders) => {
                    if let Poll::Ready(resp) = fut.poll_unpin(cx) {
                        match resp {
                            Ok((fork_id, fork, handler)) => {
                                if let Some(fork) = this.forks.get(&fork_id).cloned() {
                                    this.insert_new_fork(
                                        fork.inc_senders(fork_id),
                                        fork,
                                        sender,
                                        additional_senders,
                                    );
                                } else {
                                    this.handlers.push((fork_id.clone(), handler));
                                    this.insert_new_fork(fork_id, fork, sender, additional_senders);
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
                        this.pending_tasks.push(ForkTask::Create(
                            fut,
                            id,
                            sender,
                            additional_senders,
                        ));
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
            let forks = this.forks.values().map(|f| f.backend.clone()).collect::<Vec<_>>();
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
    /// The resolved EVM environment (fetched from the provider).
    evm_env: EvmEnv<SPEC, BLOCK>,
    /// Copy of the sender.
    backend: SharedBackend<N, BLOCK>,
    /// How many consumers there are, since a `SharedBacked` can be used by multiple
    /// consumers.
    num_senders: Arc<AtomicUsize>,
}

impl<N: Network, SPEC, BLOCK: ForkBlockEnv> CreatedFork<N, SPEC, BLOCK> {
    pub fn new(
        opts: CreateFork,
        evm_env: EvmEnv<SPEC, BLOCK>,
        backend: SharedBackend<N, BLOCK>,
    ) -> Self {
        Self { opts, evm_env, backend, num_senders: Arc::new(AtomicUsize::new(1)) }
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
/// This will establish a new `Provider` to the endpoint and return the Fork Backend.
async fn create_fork<
    N: Network,
    SPEC: Into<SpecId> + Default + Copy,
    BLOCK: FoundryBlock + ForkBlockEnv + Default,
>(
    mut fork: CreateFork,
    expected_identity: Option<ForkContext>,
) -> eyre::Result<(ForkId, CreatedFork<N, SPEC, BLOCK>, BackendHandler<N, BLOCK>)> {
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
    let any_provider = fork.evm_opts.fork_provider_with_url::<AnyNetwork>(&fork.url)?;
    let (evm_env, resolved) = if let Some(resolved) = fork.resolved.clone() {
        let evm_env = fork
            .evm_opts
            .fork_evm_env_at_resolved::<_, BLOCK, _, _>(&any_provider, &resolved)
            .await?;
        (evm_env, resolved)
    } else {
        let (evm_env, resolved) =
            fork.evm_opts.fork_evm_env_resolved::<_, BLOCK, _, _>(&any_provider).await?;
        (evm_env, resolved)
    };
    let fork_context = resolved.context();
    if require_endpoint_family_match
        && !execution_networks.supports_fork_source(&fork_context.network_profile)
    {
        eyre::bail!(
            "cannot create a `{}` fork with an EVM instantiated for `{}`",
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
    let number = resolved.number();
    let meta = BlockchainDbMeta::new(evm_env.block_env.clone(), fork.url.clone())
        .with_fork_identity(resolved.hash(), resolved.source_id());

    // Determine the cache path if caching is enabled.
    let cache_path = if fork.enable_caching {
        Config::foundry_block_cache_dir(fork_context.source_chain_id, number)
    } else {
        None
    };

    let provider = fork.evm_opts.fork_provider_with_url::<N>(&fork.url)?;
    let db = BlockchainDb::new(meta, cache_path);
    let anchor = ForkBlock::with_rpc_number(
        evm_env.block_env.number().saturating_to(),
        resolved.number(),
        resolved.hash(),
    );
    let (backend, handler) = SharedBackend::new_with_anchor(provider, db, anchor)?;
    let fork_id = ForkId::resolved(&fork.url, &resolved);
    fork.resolved = Some(resolved);
    let fork = CreatedFork::new(fork, evm_env, backend);

    Ok((fork_id, fork, handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use foundry_evm_networks::{NetworkConfigs, NetworkVariant};

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
        let first = ResolvedFork::new(
            url,
            None,
            None,
            Some(1),
            BlockNumHash::new(1, B256::with_last_byte(1)),
            context(1),
        );
        let replacement = ResolvedFork::new(
            url,
            None,
            None,
            Some(1),
            BlockNumHash::new(1, B256::with_last_byte(2)),
            context(1),
        );
        let authenticated = ResolvedFork::new(
            url,
            Some(&["Authorization: secret".to_string()]),
            None,
            Some(1),
            BlockNumHash::new(1, B256::with_last_byte(1)),
            context(1),
        );

        assert_ne!(ForkId::resolved(url, &first), ForkId::resolved(url, &replacement));
        assert_ne!(ForkId::resolved(url, &first), ForkId::resolved(url, &authenticated));
    }
}
