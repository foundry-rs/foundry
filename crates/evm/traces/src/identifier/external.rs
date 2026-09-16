use super::{IdentifiedAddress, TraceIdentifier};
use crate::debug::ContractSources;
use alloy_json_abi::JsonAbi;
use alloy_primitives::{
    Address,
    map::{AddressSet, Entry, HashMap, HashSet},
};
use eyre::WrapErr;
use foundry_block_explorers::{contract::Metadata, errors::EtherscanError};
use foundry_common::compile::etherscan_project;
use foundry_config::{Chain, Config};
use futures::{
    future::join_all,
    stream::{FuturesUnordered, Stream, StreamExt},
    task::{Context, Poll},
};
use revm_inspectors::tracing::types::CallTraceNode;
use serde::Deserialize;
use std::{
    borrow::Cow,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{Notify, Semaphore},
    time::Duration,
};

/// A trace identifier that tries to identify addresses using Etherscan.
pub struct ExternalIdentifier {
    fetchers: Arc<[Arc<SharedFetcher>]>,
    /// Lookup results, shared with every [`ExternalPrefetcher`] created from this identifier.
    cache: Arc<Cache>,
    /// Time a single background prefetch may run before it is abandoned.
    prefetch_timeout: Duration,
}

impl ExternalIdentifier {
    /// Creates a new external identifier with the given client
    pub fn new(config: &Config, mut chain: Option<Chain>) -> eyre::Result<Option<Self>> {
        let timeout = config.tracing.external_identification_timeout;
        if config.offline || timeout == 0 {
            return Ok(None);
        }

        let no_proxy = config.eth_rpc_no_proxy;
        let config = match config.get_etherscan_config_with_chain(chain) {
            Ok(Some(config)) => {
                chain = config.chain;
                Some(config)
            }
            Ok(None) => {
                warn!(target: "evm::traces::external", "etherscan config not found");
                None
            }
            Err(err) => {
                warn!(target: "evm::traces::external", ?err, "failed to get etherscan config");
                None
            }
        };

        let mut fetchers = Vec::<Arc<dyn ExternalFetcherT>>::new();
        if let Some(chain) = chain {
            debug!(target: "evm::traces::external", ?chain, "using sourcify identifier");
            fetchers.push(Arc::new(SourcifyFetcher::new(chain)));
        }
        if let Some(config) = config {
            debug!(target: "evm::traces::external", chain=?config.chain, url=?config.api_url, "using etherscan identifier");
            match config.into_client_with_no_proxy(no_proxy) {
                Ok(client) => {
                    fetchers.push(Arc::new(EtherscanFetcher::new(client)));
                }
                Err(err) => {
                    warn!(target: "evm::traces::external", ?err, "failed to create etherscan client");
                }
            }
        }
        if fetchers.is_empty() {
            debug!(target: "evm::traces::external", "no fetchers enabled");
            return Ok(None);
        }

        let timeout = Duration::from_secs(timeout);
        Ok(Some(Self::with_fetchers(fetchers, timeout)))
    }

    fn with_fetchers(fetchers: Vec<Arc<dyn ExternalFetcherT>>, timeout: Duration) -> Self {
        Self {
            fetchers: fetchers.into_iter().map(SharedFetcher::new).collect(),
            cache: Arc::new(Cache::new(timeout)),
            prefetch_timeout: timeout,
        }
    }

    /// Returns a handle that schedules lookups on `handle` ahead of [`identify_addresses`].
    ///
    /// [`identify_addresses`]: TraceIdentifier::identify_addresses
    pub fn prefetcher(&self, handle: tokio::runtime::Handle) -> ExternalPrefetcher {
        ExternalPrefetcher {
            fetchers: Arc::clone(&self.fetchers),
            cache: Arc::clone(&self.cache),
            timeout: self.prefetch_timeout,
            handle,
        }
    }

    /// Returns the remaining time external identification may block trace rendering.
    pub fn remaining_budget(&self) -> Duration {
        self.cache.state().remaining_budget
    }

    /// Goes over the list of contracts we have pulled from the traces, clones their source from
    /// Etherscan and compiles them locally, for usage in the debugger.
    pub async fn get_compiled_contracts(&self) -> eyre::Result<ContractSources> {
        // Collect contract info upfront so we can reference it in error messages
        let contracts_info: Vec<_> = self
            .cache
            .state()
            .contracts
            .iter()
            // filter out vyper files and contracts without metadata
            .filter_map(|(addr, (_, metadata))| {
                if let Some(metadata) = metadata.as_ref()
                    && !metadata.is_vyper()
                {
                    Some((*addr, metadata.clone()))
                } else {
                    None
                }
            })
            .collect();

        let outputs_fut = contracts_info
            .iter()
            .map(|(addr, metadata)| async move {
                sh_println!("Compiling: {} {addr}", metadata.contract_name)?;
                let root = tempfile::tempdir()?;
                let root_path = root.path();
                let project = etherscan_project(metadata, root_path)?;
                let output = project.compile()?;
                if output.has_compiler_errors() {
                    eyre::bail!("{output}");
                }

                Ok((project, output, root))
            })
            .collect::<Vec<_>>();

        // poll all the futures concurrently
        let outputs = join_all(outputs_fut).await;

        let mut sources: ContractSources = Default::default();

        // construct the map
        for (idx, res) in outputs.into_iter().enumerate() {
            let (addr, metadata) = &contracts_info[idx];
            let name = &metadata.contract_name;
            let (project, output, _) =
                res.wrap_err_with(|| format!("Failed to compile contract {name} at {addr}"))?;
            sources
                .insert(&output, project.root(), None)
                .wrap_err_with(|| format!("Failed to insert contract {name} at {addr}"))?;
        }

        Ok(sources)
    }

    fn identify_from_metadata(address: Address, metadata: &Metadata) -> IdentifiedAddress<'static> {
        let label = metadata.contract_name.clone();
        let abi = metadata.abi().ok().map(Cow::Owned);
        IdentifiedAddress {
            address,
            label: Some(label.clone()),
            contract: Some(label),
            abi,
            constructor_args_offset: None,
            artifact_id: None,
        }
    }

    /// Fully resolves `addresses`, blocking for at most the remaining budget.
    ///
    /// Addresses with a prefetch in flight are awaited rather than fetched again. Time spent here
    /// is charged to the budget, and a timeout spends all of it.
    async fn fetch_addresses_async(&mut self, addresses: &[Address]) {
        let budget = self.remaining_budget();
        if addresses.is_empty() || budget.is_zero() {
            return;
        }

        let started = tokio::time::Instant::now();
        let timed_out = tokio::time::timeout(budget, async {
            loop {
                let (claim, to_await) = self.cache.claim(addresses);
                if claim.addresses.is_empty() && to_await.is_empty() {
                    break;
                }
                futures::future::join(
                    fetch_batch(&self.fetchers, claim),
                    self.cache.settled(&to_await),
                )
                .await;
            }
        })
        .await
        .is_err();
        self.cache.spend(started.elapsed(), timed_out);
    }

    /// Fetches all verified ABIs and whether each proxy chain was fully resolved.
    pub async fn get_abis(
        &mut self,
        addresses: &[Address],
    ) -> Vec<(Address, eyre::Result<(Vec<JsonAbi>, bool)>)> {
        const MAX_PROXY_DEPTH: usize = 16;

        struct Chain {
            current: Option<Address>,
            visited: HashSet<Address>,
            abis: Vec<JsonAbi>,
            complete: bool,
        }

        let mut chains = addresses
            .iter()
            .map(|&address| Chain {
                current: Some(address),
                visited: HashSet::default(),
                abis: Vec::new(),
                complete: true,
            })
            .collect::<Vec<_>>();

        for _ in 0..MAX_PROXY_DEPTH {
            let to_fetch = {
                let state = self.cache.state();
                chains
                    .iter()
                    .filter_map(|chain| chain.current)
                    .filter(|address| !state.completed.contains(address))
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            };
            self.fetch_addresses_async(&to_fetch).await;

            let mut has_next = false;
            let state = self.cache.state();
            for chain in &mut chains {
                let Some(current) = chain.current else { continue };
                if !chain.visited.insert(current) {
                    chain.current = None;
                    chain.complete = false;
                    continue;
                }
                let Some((_, Some(metadata))) = state.contracts.get(&current) else {
                    chain.current = None;
                    chain.complete = false;
                    continue;
                };
                if let Ok(abi) = metadata.abi() {
                    chain.abis.push(abi);
                } else {
                    chain.complete = false;
                }
                chain.current = (metadata.proxy != 0).then_some(metadata.implementation).flatten();
                if metadata.proxy != 0 && chain.current.is_none() {
                    chain.complete = false;
                }
                has_next |= chain.current.is_some();
            }
            if !has_next {
                break;
            }
        }

        chains
            .into_iter()
            .zip(addresses.iter().copied())
            .map(|(mut chain, address)| {
                chain.complete &= chain.current.is_none();
                let result = if chain.abis.is_empty() {
                    Err(eyre::eyre!("external ABI lookup failed"))
                } else {
                    Ok((chain.abis.into_iter().rev().collect(), chain.complete))
                };
                (address, result)
            })
            .collect()
    }
}

impl TraceIdentifier for ExternalIdentifier {
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        if nodes.is_empty() {
            return Vec::new();
        }

        trace!(target: "evm::traces::external", "identify {} addresses", nodes.len());

        let addresses = nodes.iter().map(|node| node.trace.address).collect::<AddressSet>();
        let mut identities = Vec::with_capacity(addresses.len());
        let mut unresolved = Vec::new();

        // Check cache first.
        {
            let state = self.cache.state();
            for address in addresses {
                if !state.completed.contains(&address) {
                    unresolved.push(address);
                } else if let Some((_, metadata)) = state.contracts.get(&address) {
                    if let Some(metadata) = metadata {
                        identities.push(Self::identify_from_metadata(address, metadata));
                    } else {
                        // Do nothing. We know that this contract was not verified.
                    }
                } else {
                    unresolved.push(address);
                }
            }
        }

        if unresolved.is_empty() {
            return identities;
        }
        if !self.remaining_budget().is_zero() {
            trace!(target: "evm::traces::external", "fetching {} addresses", unresolved.len());
            foundry_common::block_on(self.fetch_addresses_async(&unresolved));
        }

        let state = self.cache.state();
        for address in unresolved {
            if let Some((_, Some(metadata))) = state.contracts.get(&address) {
                identities.push(Self::identify_from_metadata(address, metadata));
            }
        }
        trace!(target: "evm::traces::external", "identified {} addresses", identities.len());
        identities
    }
}

/// Schedules external lookups before the traces that need them reach the identifier.
///
/// Results land in the cache shared with the [`ExternalIdentifier`] this handle was created from,
/// so a later [`identify_addresses`] call for the same addresses finds them, or waits for the
/// in-flight lookup, instead of fetching again. Nothing is scheduled once the identifier's budget
/// is spent.
///
/// [`identify_addresses`]: TraceIdentifier::identify_addresses
#[derive(Clone)]
pub struct ExternalPrefetcher {
    fetchers: Arc<[Arc<SharedFetcher>]>,
    cache: Arc<Cache>,
    /// Time a single prefetch may run before it is abandoned.
    timeout: Duration,
    /// The runtime prefetches are spawned on.
    handle: tokio::runtime::Handle,
}

impl std::fmt::Debug for ExternalPrefetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalPrefetcher").field("timeout", &self.timeout).finish_non_exhaustive()
    }
}

impl ExternalPrefetcher {
    /// Starts fetching `addresses` that are neither cached nor already in flight.
    pub fn prefetch(&self, addresses: impl IntoIterator<Item = Address>) {
        let Some(claim) = self.cache.claim_new(addresses) else {
            return;
        };
        trace!(target: "evm::traces::external", "prefetching {} addresses", claim.addresses.len());

        let fetchers = Arc::clone(&self.fetchers);
        let timeout = self.timeout;
        self.handle.spawn(async move {
            if tokio::time::timeout(timeout, fetch_batch(&fetchers, claim)).await.is_err() {
                warn!(target: "evm::traces::external", "external prefetch timed out");
            }
        });
    }
}

/// Lookup results shared between an [`ExternalIdentifier`] and its [`ExternalPrefetcher`]s.
struct Cache {
    state: Mutex<CacheState>,
    /// Signalled whenever an in-flight lookup settles.
    notify: Notify,
}

#[derive(Default)]
struct CacheState {
    /// Fetched contracts, including negative results.
    contracts: HashMap<Address, (FetcherKind, Option<Metadata>)>,
    /// Addresses with a lookup in progress.
    in_flight: AddressSet,
    /// Addresses for which every fetcher has finished.
    completed: AddressSet,
    /// Remaining time external identification may block trace rendering.
    remaining_budget: Duration,
}

impl Cache {
    fn new(budget: Duration) -> Self {
        Self {
            state: Mutex::new(CacheState { remaining_budget: budget, ..Default::default() }),
            notify: Notify::new(),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Splits unresolved `addresses` into the ones to fetch now, marking them in flight, and the
    /// ones with a lookup already in flight. Completed addresses are omitted.
    fn claim(self: &Arc<Self>, addresses: &[Address]) -> (Claim, Vec<Address>) {
        let mut state = self.state();
        let mut to_fetch = Vec::new();
        let mut to_await = Vec::new();
        for &address in addresses {
            if state.in_flight.contains(&address) {
                to_await.push(address);
            } else if !state.completed.contains(&address) {
                state.in_flight.insert(address);
                to_fetch.push(address);
            }
        }
        drop(state);
        (Claim { cache: Arc::clone(self), addresses: to_fetch }, to_await)
    }

    /// Marks the addresses of `addresses` that are neither cached nor in flight as in flight and
    /// returns them. Returns nothing once the budget is spent.
    fn claim_new(self: &Arc<Self>, addresses: impl IntoIterator<Item = Address>) -> Option<Claim> {
        let mut state = self.state();
        if state.remaining_budget.is_zero() {
            return None;
        }
        let addresses = addresses
            .into_iter()
            .filter(|address| {
                !state.completed.contains(address) && state.in_flight.insert(*address)
            })
            .collect::<Vec<_>>();
        drop(state);
        if addresses.is_empty() { None } else { Some(Claim { cache: Arc::clone(self), addresses }) }
    }

    /// Records one provider's result and settles the address when it is the last provider.
    fn record(&self, address: Address, value: (FetcherKind, Option<Metadata>), complete: bool) {
        let mut state = self.state();
        match state.contracts.entry(address) {
            Entry::Occupied(mut occupied_entry) => {
                let old = occupied_entry.get();
                // Only override when the new result is strictly better:
                // - new has metadata and old doesn't, OR
                // - both have metadata but new is from Etherscan and old is not.
                // Never downgrade a successful lookup to None.
                let should_replace = match (&old.1, &value.1) {
                    (None, Some(_)) => true,
                    (Some(_), None) => false,
                    _ => {
                        matches!(value.0, FetcherKind::Etherscan)
                            && !matches!(old.0, FetcherKind::Etherscan)
                    }
                };
                if should_replace {
                    occupied_entry.insert(value);
                }
            }
            Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(value);
            }
        }
        if complete {
            state.contracts.entry(address).or_insert((FetcherKind::Sourcify, None));
            state.in_flight.remove(&address);
            state.completed.insert(address);
        }
        drop(state);
        if complete {
            self.notify.notify_waiters();
        }
    }

    /// Releases in-flight marks for lookups that ended without a result.
    fn release(&self, addresses: &[Address]) {
        if addresses.is_empty() {
            return;
        }
        let mut state = self.state();
        for address in addresses {
            state.in_flight.remove(address);
        }
        drop(state);
        self.notify.notify_waiters();
    }

    /// Resolves once none of `addresses` has a lookup in flight.
    async fn settled(&self, addresses: &[Address]) {
        loop {
            let notified = self.notify.notified();
            let settled = {
                let state = self.state();
                addresses.iter().all(|address| !state.in_flight.contains(address))
            };
            if settled {
                return;
            }
            notified.await;
        }
    }

    /// Charges `elapsed` to the budget; a timeout spends all of it.
    fn spend(&self, elapsed: Duration, timed_out: bool) {
        let mut state = self.state();
        state.remaining_budget = state.remaining_budget.saturating_sub(elapsed);
        if timed_out {
            state.remaining_budget = Duration::ZERO;
            warn!(target: "evm::traces::external", "external identification timed out; disabling it for the remainder of this session");
        }
    }
}

/// Releases every unfinished address if its lookup is dropped.
struct Claim {
    cache: Arc<Cache>,
    addresses: Vec<Address>,
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.cache.release(&self.addresses);
    }
}

/// Runs every fetcher over `addresses` and records each result in `cache`.
///
/// Addresses settle after every fetcher finishes. If this future is dropped, its in-flight marks
/// are released so a cancelled batch never leaves a waiter behind.
async fn fetch_batch(fetchers: &[Arc<SharedFetcher>], mut claim: Claim) {
    if claim.addresses.is_empty() {
        return;
    }
    if fetchers.is_empty() {
        for &address in &claim.addresses {
            claim.cache.record(address, (FetcherKind::Sourcify, None), true);
        }
        claim.addresses.clear();
        return;
    }
    let mut remaining =
        claim.addresses.iter().map(|&address| (address, fetchers.len())).collect::<HashMap<_, _>>();
    let mut unfinished = claim.addresses.len();
    let fetchers =
        fetchers.iter().map(|fetcher| ExternalFetcher::new(Arc::clone(fetcher), &claim.addresses));
    let mut fetched = futures::stream::select_all(fetchers);
    while let Some((address, value)) = fetched.next().await {
        let providers = remaining.get_mut(&address).expect("fetcher yielded an unknown address");
        *providers -= 1;
        let complete = *providers == 0;
        claim.cache.record(address, value, complete);
        if complete {
            unfinished -= 1;
        }
    }
    if unfinished == 0 {
        claim.addresses.clear();
    }
}

type FetchFuture =
    Pin<Box<dyn Future<Output = (Address, Result<Option<Metadata>, EtherscanError>)> + Send>>;

/// Maximum number of times a single address is retried through a transient Cloudflare block before
/// we give up on it. Bounded so a persistent block can't loop forever.
const MAX_CLOUDFLARE_RETRIES: u32 = 5;

/// Shared concurrency and backoff state for one external provider.
struct SharedFetcher {
    provider: Arc<dyn ExternalFetcherT>,
    permits: Semaphore,
    backoff_until: Mutex<Option<tokio::time::Instant>>,
}

impl SharedFetcher {
    fn new(provider: Arc<dyn ExternalFetcherT>) -> Arc<Self> {
        Arc::new(Self {
            permits: Semaphore::new(provider.concurrency()),
            provider,
            backoff_until: Mutex::new(None),
        })
    }

    async fn fetch(&self, address: Address) -> Result<Option<Metadata>, EtherscanError> {
        loop {
            let backoff_deadline =
                *self.backoff_until.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(deadline) = backoff_deadline
                && deadline > tokio::time::Instant::now()
            {
                tokio::time::sleep_until(deadline).await;
            }

            let permit = self.permits.acquire().await.expect("fetcher semaphore is never closed");
            let backoff_deadline =
                *self.backoff_until.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(deadline) = backoff_deadline
                && deadline > tokio::time::Instant::now()
            {
                drop(permit);
                tokio::time::sleep_until(deadline).await;
                continue;
            }

            let result = self.provider.fetch(address).await;
            if matches!(
                result,
                Err(EtherscanError::RateLimitExceeded | EtherscanError::BlockedByCloudflare)
            ) {
                let deadline = tokio::time::Instant::now() + self.provider.timeout();
                let mut backoff =
                    self.backoff_until.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if backoff.is_none_or(|current| current < deadline) {
                    *backoff = Some(deadline);
                }
            }
            return result;
        }
    }
}

/// A rate limit aware fetcher.
///
/// Fetches information about multiple addresses concurrently, while respecting rate limits.
struct ExternalFetcher {
    /// The fetcher
    fetcher: Arc<SharedFetcher>,
    /// The maximum amount of requests to send concurrently
    concurrency: usize,
    /// The addresses we have yet to make requests for
    queue: Vec<Address>,
    /// The in progress requests
    in_progress: FuturesUnordered<FetchFuture>,
    /// Per-address retry counter for transient Cloudflare blocks.
    attempts: HashMap<Address, u32>,
    /// Addresses for which this provider has not produced a terminal result.
    pending: AddressSet,
}

impl ExternalFetcher {
    fn new(fetcher: Arc<SharedFetcher>, to_fetch: &[Address]) -> Self {
        Self {
            concurrency: fetcher.provider.concurrency(),
            fetcher,
            queue: to_fetch.to_vec(),
            in_progress: FuturesUnordered::new(),
            attempts: HashMap::default(),
            pending: to_fetch.iter().copied().collect(),
        }
    }

    fn queue_next_reqs(&mut self) {
        while self.in_progress.len() < self.concurrency {
            let Some(addr) = self.queue.pop() else { break };
            let fetcher = Arc::clone(&self.fetcher);
            self.in_progress.push(Box::pin(async move {
                trace!(target: "evm::traces::external", ?addr, "fetching info");
                let res = fetcher.fetch(addr).await;
                (addr, res)
            }));
        }
    }
}

impl Stream for ExternalFetcher {
    type Item = (Address, (FetcherKind, Option<Metadata>));

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        let kind = pin.fetcher.provider.kind();

        let _guard = info_span!("evm::traces::external", ?kind, "ExternalFetcher").entered();

        if pin.fetcher.provider.invalid_api_key().load(Ordering::Relaxed) {
            pin.queue.clear();
            pin.in_progress.clear();
            let Some(address) = pin.pending.iter().next().copied() else {
                return Poll::Ready(None);
            };
            pin.pending.remove(&address);
            return Poll::Ready(Some((address, (kind, None))));
        }

        loop {
            pin.queue_next_reqs();

            let mut made_progress_this_iter = false;
            match pin.in_progress.poll_next_unpin(cx) {
                Poll::Pending => {}
                Poll::Ready(None) => {
                    debug_assert!(pin.pending.is_empty());
                    return Poll::Ready(None);
                }
                Poll::Ready(Some((addr, res))) => {
                    made_progress_this_iter = true;
                    match res {
                        Ok(metadata) => {
                            pin.pending.remove(&addr);
                            return Poll::Ready(Some((addr, (kind, metadata))));
                        }
                        Err(EtherscanError::ContractCodeNotVerified(_)) => {
                            pin.pending.remove(&addr);
                            return Poll::Ready(Some((addr, (kind, None))));
                        }
                        Err(EtherscanError::RateLimitExceeded) => {
                            warn!(target: "evm::traces::external", "rate limit exceeded on attempt");
                            pin.queue.push(addr);
                        }
                        Err(EtherscanError::InvalidApiKey) => {
                            warn!(target: "evm::traces::external", "invalid api key");
                            // mark key as invalid
                            pin.fetcher.provider.invalid_api_key().store(true, Ordering::Relaxed);
                            pin.pending.remove(&addr);
                            return Poll::Ready(Some((addr, (kind, None))));
                        }
                        Err(EtherscanError::BlockedByCloudflare) => {
                            // A Cloudflare block is transient rate limiting (often triggered
                            // by request bursts), not a permanent failure like an invalid key.
                            // Back off and retry the address a bounded number of times instead
                            // of aborting the whole stream, which would abandon every still-
                            // queued address and leave traces only partially decoded (#9880).
                            let attempts = {
                                let entry = pin.attempts.entry(addr).or_default();
                                *entry += 1;
                                *entry
                            };
                            if attempts <= MAX_CLOUDFLARE_RETRIES {
                                warn!(target: "evm::traces::external", attempts, "blocked by cloudflare, backing off");
                                pin.queue.push(addr);
                            } else {
                                warn!(target: "evm::traces::external", "blocked by cloudflare, giving up on address");
                                pin.pending.remove(&addr);
                                return Poll::Ready(Some((addr, (kind, None))));
                            }
                        }
                        Err(err) => {
                            warn!(target: "evm::traces::external", ?err, "could not get info");
                            // Cache the failure so we don't re-fetch on subsequent arenas.
                            pin.pending.remove(&addr);
                            return Poll::Ready(Some((addr, (kind, None))));
                        }
                    }
                }
            }

            if !made_progress_this_iter {
                return Poll::Pending;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetcherKind {
    Etherscan,
    Sourcify,
}

#[async_trait::async_trait]
trait ExternalFetcherT: Send + Sync {
    fn kind(&self) -> FetcherKind;
    fn timeout(&self) -> Duration;
    fn concurrency(&self) -> usize;
    fn invalid_api_key(&self) -> &AtomicBool;
    async fn fetch(&self, address: Address) -> Result<Option<Metadata>, EtherscanError>;
}

struct EtherscanFetcher {
    client: foundry_block_explorers::Client,
    invalid_api_key: AtomicBool,
}

impl EtherscanFetcher {
    const fn new(client: foundry_block_explorers::Client) -> Self {
        Self { client, invalid_api_key: AtomicBool::new(false) }
    }
}

#[async_trait::async_trait]
impl ExternalFetcherT for EtherscanFetcher {
    fn kind(&self) -> FetcherKind {
        FetcherKind::Etherscan
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn concurrency(&self) -> usize {
        5
    }

    fn invalid_api_key(&self) -> &AtomicBool {
        &self.invalid_api_key
    }

    async fn fetch(&self, address: Address) -> Result<Option<Metadata>, EtherscanError> {
        self.client.contract_source_code(address).await.map(|mut metadata| metadata.items.pop())
    }
}

struct SourcifyFetcher {
    client: reqwest::Client,
    url: String,
    invalid_api_key: AtomicBool,
}

impl SourcifyFetcher {
    fn new(chain: Chain) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(foundry_common::DEFAULT_USER_AGENT)
                .build()
                .expect("Client::builder() with static config cannot fail"),
            url: format!("https://sourcify.dev/server/v2/contract/{}", chain.id()),
            invalid_api_key: AtomicBool::new(false),
        }
    }
}

#[async_trait::async_trait]
impl ExternalFetcherT for SourcifyFetcher {
    fn kind(&self) -> FetcherKind {
        FetcherKind::Sourcify
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn concurrency(&self) -> usize {
        5
    }

    fn invalid_api_key(&self) -> &AtomicBool {
        &self.invalid_api_key
    }

    async fn fetch(&self, address: Address) -> Result<Option<Metadata>, EtherscanError> {
        let url = format!("{url}/{address}?fields=abi,compilation", url = self.url);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| EtherscanError::Unknown(e.to_string()))?;
        let code = response.status();
        match code.as_u16() {
            // Not verified.
            404 => return Err(EtherscanError::ContractCodeNotVerified(address)),
            // Too many requests.
            429 => return Err(EtherscanError::RateLimitExceeded),
            _ => {}
        }
        let response: SourcifyResponse =
            response.json().await.map_err(|e| EtherscanError::Unknown(e.to_string()))?;
        trace!(target: "evm::traces::external", "Sourcify response for {address}: {response:#?}");
        match response {
            SourcifyResponse::Success(metadata) => Ok(Some(metadata.into())),
            SourcifyResponse::Error(error) => Err(EtherscanError::Unknown(format!("{error:#?}"))),
        }
    }
}

/// Sourcify API response for `/v2/contract/{chainId}/{address}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum SourcifyResponse {
    Success(SourcifyMetadata),
    Error(SourcifyError),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(dead_code)] // Used in Debug.
struct SourcifyError {
    custom_code: String,
    message: String,
    error_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcifyMetadata {
    #[serde(default)]
    abi: Option<Box<serde_json::value::RawValue>>,
    #[serde(default)]
    compilation: Option<Compilation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Compilation {
    #[serde(default)]
    compiler_version: String,
    #[serde(default)]
    name: String,
}

impl From<SourcifyMetadata> for Metadata {
    fn from(metadata: SourcifyMetadata) -> Self {
        let SourcifyMetadata { abi, compilation } = metadata;
        let (contract_name, compiler_version) = compilation
            .map(|c| (c.name, c.compiler_version))
            .unwrap_or_else(|| (String::new(), String::new()));
        // Defaulted fields may be fetched from sourcify but we don't make use of them.
        Self {
            source_code: foundry_block_explorers::contract::SourceCodeMetadata::Sources(
                Default::default(),
            ),
            abi: Box::<str>::from(abi.unwrap_or_default()).into(),
            contract_name,
            compiler_version,
            optimization_used: 0,
            runs: 0,
            constructor_arguments: Default::default(),
            evm_version: String::new(),
            library: String::new(),
            license_type: String::new(),
            proxy: 0,
            implementation: None,
            swarm_source: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet as StdHashSet,
        future::pending,
        sync::atomic::{AtomicUsize, Ordering as AtomicOrdering},
    };

    struct TestFetcher {
        kind: FetcherKind,
        delay: Option<Duration>,
        contract_name: Option<&'static str>,
        calls: Arc<AtomicUsize>,
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for TestFetcher {
        fn kind(&self) -> FetcherKind {
            self.kind
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(1)
        }

        fn concurrency(&self) -> usize {
            1
        }

        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }

        async fn fetch(&self, _address: Address) -> Result<Option<Metadata>, EtherscanError> {
            self.calls.fetch_add(1, AtomicOrdering::Relaxed);
            let Some(delay) = self.delay else { return pending().await };
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            Ok(self.contract_name.map(metadata))
        }
    }

    struct RateLimitedFetcher {
        calls: Arc<AtomicUsize>,
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for RateLimitedFetcher {
        fn kind(&self) -> FetcherKind {
            FetcherKind::Sourcify
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(5)
        }

        fn concurrency(&self) -> usize {
            1
        }

        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }

        async fn fetch(&self, _address: Address) -> Result<Option<Metadata>, EtherscanError> {
            self.calls.fetch_add(1, AtomicOrdering::Relaxed);
            Err(EtherscanError::RateLimitExceeded)
        }
    }

    struct ConcurrentFetcher {
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for ConcurrentFetcher {
        fn kind(&self) -> FetcherKind {
            FetcherKind::Sourcify
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(1)
        }

        fn concurrency(&self) -> usize {
            2
        }

        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }

        async fn fetch(&self, _address: Address) -> Result<Option<Metadata>, EtherscanError> {
            self.calls.fetch_add(1, AtomicOrdering::Relaxed);
            let active = self.active.fetch_add(1, AtomicOrdering::Relaxed) + 1;
            self.max_active.fetch_max(active, AtomicOrdering::Relaxed);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.active.fetch_sub(1, AtomicOrdering::Relaxed);
            Ok(None)
        }
    }

    struct OnceRateLimitedFetcher {
        calls: Arc<AtomicUsize>,
        first_response: Arc<Notify>,
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for OnceRateLimitedFetcher {
        fn kind(&self) -> FetcherKind {
            FetcherKind::Sourcify
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(50)
        }

        fn concurrency(&self) -> usize {
            1
        }

        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }

        async fn fetch(&self, _address: Address) -> Result<Option<Metadata>, EtherscanError> {
            if self.calls.fetch_add(1, AtomicOrdering::Relaxed) == 0 {
                self.first_response.notified().await;
                Err(EtherscanError::RateLimitExceeded)
            } else {
                Ok(None)
            }
        }
    }

    struct AbandonedFetcher {
        kind: FetcherKind,
        calls: Arc<AtomicUsize>,
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for AbandonedFetcher {
        fn kind(&self) -> FetcherKind {
            self.kind
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(1)
        }

        fn concurrency(&self) -> usize {
            1
        }

        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }

        async fn fetch(&self, _address: Address) -> Result<Option<Metadata>, EtherscanError> {
            if self.calls.fetch_add(1, AtomicOrdering::Relaxed) == 0 {
                pending().await
            } else {
                Ok(Some(metadata("Retried")))
            }
        }
    }

    struct AddressFetcher {
        stalled: Address,
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for AddressFetcher {
        fn kind(&self) -> FetcherKind {
            FetcherKind::Sourcify
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(1)
        }

        fn concurrency(&self) -> usize {
            1
        }

        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }

        async fn fetch(&self, address: Address) -> Result<Option<Metadata>, EtherscanError> {
            if address == self.stalled { pending().await } else { Ok(Some(metadata("Ready"))) }
        }
    }

    struct InvalidKeyFetcher {
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for InvalidKeyFetcher {
        fn kind(&self) -> FetcherKind {
            FetcherKind::Etherscan
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(1)
        }

        fn concurrency(&self) -> usize {
            1
        }

        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }

        async fn fetch(&self, _address: Address) -> Result<Option<Metadata>, EtherscanError> {
            Err(EtherscanError::InvalidApiKey)
        }
    }

    fn metadata(contract_name: &str) -> Metadata {
        SourcifyMetadata {
            abi: None,
            compilation: Some(Compilation {
                compiler_version: String::new(),
                name: contract_name.to_string(),
            }),
        }
        .into()
    }

    fn test_identifier(
        fetchers: Vec<Arc<dyn ExternalFetcherT>>,
        remaining_budget: Duration,
    ) -> ExternalIdentifier {
        ExternalIdentifier::with_fetchers(fetchers, remaining_budget)
    }

    #[test]
    fn zero_timeout_disables_external_identification() {
        let mut config = Config::default();
        config.tracing.external_identification_timeout = 0;

        assert!(ExternalIdentifier::new(&config, Some(Chain::mainnet())).unwrap().is_none());
    }

    /// Fetcher that returns a transient Cloudflare block the first time it sees an address, then
    /// succeeds. Mirrors Etherscan/Cloudflare throttling a burst of concurrent requests.
    struct FlakyCloudflareFetcher {
        seen: Mutex<StdHashSet<Address>>,
        invalid: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ExternalFetcherT for FlakyCloudflareFetcher {
        fn kind(&self) -> FetcherKind {
            FetcherKind::Etherscan
        }
        fn timeout(&self) -> Duration {
            Duration::from_millis(1)
        }
        fn concurrency(&self) -> usize {
            1
        }
        fn invalid_api_key(&self) -> &AtomicBool {
            &self.invalid
        }
        async fn fetch(&self, address: Address) -> Result<Option<Metadata>, EtherscanError> {
            let first_time = self.seen.lock().unwrap().insert(address);
            if first_time { Err(EtherscanError::BlockedByCloudflare) } else { Ok(None) }
        }
    }

    /// Regression test for #9880: a transient Cloudflare block on one address must not abandon the
    /// rest of the queue. Before the fix the fetcher returned `Poll::Ready(None)` on the first
    /// block, ending the stream and leaving later addresses unidentified (partial trace decoding).
    #[tokio::test]
    async fn cloudflare_block_retries_instead_of_abandoning_queue() {
        let addrs: Vec<Address> = (1u8..=4).map(Address::with_last_byte).collect();
        let fetcher: Arc<dyn ExternalFetcherT> = Arc::new(FlakyCloudflareFetcher {
            seen: Mutex::new(StdHashSet::new()),
            invalid: AtomicBool::new(false),
        });

        let collected: Vec<_> =
            ExternalFetcher::new(SharedFetcher::new(fetcher), &addrs).collect().await;

        let got: StdHashSet<Address> = collected.into_iter().map(|(addr, _)| addr).collect();
        let want: StdHashSet<Address> = addrs.into_iter().collect();
        assert_eq!(got, want, "every address must be yielded despite a transient cloudflare block");
    }

    #[tokio::test]
    async fn invalid_key_settles_every_queued_address() {
        let addresses = (1u8..=4).map(Address::with_last_byte).collect::<Vec<_>>();
        let fetcher: Arc<dyn ExternalFetcherT> =
            Arc::new(InvalidKeyFetcher { invalid: AtomicBool::new(false) });

        let results =
            ExternalFetcher::new(SharedFetcher::new(fetcher), &addresses).collect::<Vec<_>>().await;

        assert_eq!(results.len(), addresses.len());
        assert_eq!(
            results.into_iter().map(|(address, _)| address).collect::<StdHashSet<_>>(),
            addresses.into_iter().collect()
        );
    }

    #[tokio::test]
    async fn address_settles_without_waiting_for_stalled_batch_sibling() {
        let stalled = Address::with_last_byte(1);
        let ready = Address::with_last_byte(2);
        let fetcher: Arc<dyn ExternalFetcherT> =
            Arc::new(AddressFetcher { stalled, invalid: AtomicBool::new(false) });
        let identifier = test_identifier(vec![fetcher], Duration::from_secs(1));
        let claim = identifier.cache.claim_new([stalled, ready]).unwrap();
        let fetchers = Arc::clone(&identifier.fetchers);
        let task = tokio::spawn(async move { fetch_batch(&fetchers, claim).await });

        tokio::time::timeout(Duration::from_secs(1), identifier.cache.settled(&[ready]))
            .await
            .unwrap();

        {
            let state = identifier.cache.state();
            assert!(state.completed.contains(&ready));
            assert!(state.in_flight.contains(&stalled));
        }
        task.abort();
        assert!(task.await.is_err());
        assert!(identifier.cache.state().in_flight.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_keeps_partial_results_and_opens_circuit() {
        let successful_calls = Arc::new(AtomicUsize::new(0));
        let stalled_calls = Arc::new(AtomicUsize::new(0));
        let fetchers: Vec<Arc<dyn ExternalFetcherT>> = vec![
            Arc::new(TestFetcher {
                kind: FetcherKind::Sourcify,
                delay: Some(Duration::ZERO),
                contract_name: Some("PartialResult"),
                calls: Arc::clone(&successful_calls),
                invalid: AtomicBool::new(false),
            }),
            Arc::new(TestFetcher {
                kind: FetcherKind::Etherscan,
                delay: None,
                contract_name: None,
                calls: Arc::clone(&stalled_calls),
                invalid: AtomicBool::new(false),
            }),
        ];
        let mut identifier = test_identifier(fetchers, Duration::from_millis(20));
        let address = Address::with_last_byte(1);

        identifier.fetch_addresses_async(&[address]).await;

        assert!(identifier.remaining_budget().is_zero());
        assert!(identifier.cache.state().in_flight.is_empty());
        assert_eq!(
            identifier.cache.state().contracts[&address].1.as_ref().unwrap().contract_name,
            "PartialResult"
        );
        assert_eq!(successful_calls.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(stalled_calls.load(AtomicOrdering::Relaxed), 1);

        identifier.fetch_addresses_async(&[Address::with_last_byte(2)]).await;
        assert_eq!(successful_calls.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(stalled_calls.load(AtomicOrdering::Relaxed), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn timeout_returns_partial_identity() {
        let fetchers: Vec<Arc<dyn ExternalFetcherT>> = vec![
            Arc::new(TestFetcher {
                kind: FetcherKind::Sourcify,
                delay: Some(Duration::ZERO),
                contract_name: Some("PartialResult"),
                calls: Arc::new(AtomicUsize::new(0)),
                invalid: AtomicBool::new(false),
            }),
            Arc::new(TestFetcher {
                kind: FetcherKind::Etherscan,
                delay: None,
                contract_name: None,
                calls: Arc::new(AtomicUsize::new(0)),
                invalid: AtomicBool::new(false),
            }),
        ];
        let mut identifier = test_identifier(fetchers, Duration::from_millis(20));
        let mut node = CallTraceNode::default();
        node.trace.address = Address::with_last_byte(1);

        let identities = identifier.identify_addresses(&[&node]);

        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].label.as_deref(), Some("PartialResult"));
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_budget_is_cumulative_across_fetches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher: Arc<dyn ExternalFetcherT> = Arc::new(TestFetcher {
            kind: FetcherKind::Sourcify,
            delay: Some(Duration::from_millis(20)),
            contract_name: Some("FirstResult"),
            calls: Arc::clone(&calls),
            invalid: AtomicBool::new(false),
        });
        let mut identifier = test_identifier(vec![fetcher], Duration::from_millis(30));
        let first = Address::with_last_byte(1);
        let second = Address::with_last_byte(2);

        identifier.fetch_addresses_async(&[first]).await;
        assert!(identifier.cache.state().contracts[&first].1.is_some());
        assert!(identifier.remaining_budget() < Duration::from_millis(15));

        identifier.fetch_addresses_async(&[second]).await;
        assert!(identifier.remaining_budget().is_zero());
        assert!(!identifier.cache.state().contracts.contains_key(&second));
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_retries_cannot_escape_timeout_budget() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher: Arc<dyn ExternalFetcherT> = Arc::new(RateLimitedFetcher {
            calls: Arc::clone(&calls),
            invalid: AtomicBool::new(false),
        });
        let mut identifier = test_identifier(vec![fetcher], Duration::from_millis(20));

        identifier.fetch_addresses_async(&[Address::with_last_byte(1)]).await;

        assert!(identifier.remaining_budget().is_zero());
        assert!(calls.load(AtomicOrdering::Relaxed) > 1);
    }

    #[test]
    fn etherscan_metadata_takes_precedence() {
        let address = Address::with_last_byte(1);
        let identifier = test_identifier(Vec::new(), Duration::ZERO);

        identifier.cache.record(
            address,
            (FetcherKind::Sourcify, Some(metadata("SourcifyResult"))),
            false,
        );
        identifier.cache.record(address, (FetcherKind::Etherscan, None), false);
        assert_eq!(
            identifier.cache.state().contracts[&address].1.as_ref().unwrap().contract_name,
            "SourcifyResult"
        );

        identifier.cache.record(
            address,
            (FetcherKind::Etherscan, Some(metadata("EtherscanResult"))),
            false,
        );
        assert_eq!(
            identifier.cache.state().contracts[&address].1.as_ref().unwrap().contract_name,
            "EtherscanResult"
        );
    }

    #[tokio::test]
    async fn proxy_metadata_preserves_address_identity_and_all_abis() {
        let proxy = Address::with_last_byte(1);
        let implementation_address = Address::with_last_byte(2);
        let mut proxy_metadata = metadata("Proxy");
        proxy_metadata.abi =
            r#"[{"anonymous":false,"inputs":[],"name":"ProxyEvent","type":"event"}]"#.to_string();
        proxy_metadata.proxy = 1;
        proxy_metadata.implementation = Some(implementation_address);
        let mut implementation = metadata("Implementation");
        implementation.abi =
            r#"[{"anonymous":false,"inputs":[],"name":"ImplementationEvent","type":"event"}]"#
                .to_string();
        let mut identifier = test_identifier(Vec::new(), Duration::from_secs(1));
        let identity = ExternalIdentifier::identify_from_metadata(proxy, &proxy_metadata);
        assert_eq!(identity.contract.as_deref(), Some("Proxy"));
        identifier.cache.record(proxy, (FetcherKind::Etherscan, Some(proxy_metadata)), false);
        identifier.cache.record(
            implementation_address,
            (FetcherKind::Etherscan, Some(implementation)),
            false,
        );

        let mut results = identifier.get_abis(&[proxy]).await;
        let (result_address, result) = results.pop().unwrap();
        let (abis, complete) = result.unwrap();
        let event_names =
            abis.into_iter().map(|abi| abi.events.into_keys().next().unwrap()).collect::<Vec<_>>();

        assert_eq!(result_address, proxy);
        assert!(complete);
        assert_eq!(event_names, ["ImplementationEvent", "ProxyEvent"]);

        identifier.cache.state().contracts.remove(&implementation_address);
        let (_, result) = identifier.get_abis(&[proxy]).await.pop().unwrap();
        let (abis, complete) = result.unwrap();
        assert_eq!(abis.len(), 1);
        assert!(!complete);
    }

    fn test_fetcher(
        delay: Option<Duration>,
        contract_name: Option<&'static str>,
        calls: &Arc<AtomicUsize>,
    ) -> Arc<dyn ExternalFetcherT> {
        Arc::new(TestFetcher {
            kind: FetcherKind::Sourcify,
            delay,
            contract_name,
            calls: Arc::clone(calls),
            invalid: AtomicBool::new(false),
        })
    }

    fn node(address: Address) -> CallTraceNode {
        let mut node = CallTraceNode::default();
        node.trace.address = address;
        node
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prefetched_address_is_served_from_cache() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher = test_fetcher(Some(Duration::ZERO), Some("Prefetched"), &calls);
        let mut identifier = test_identifier(vec![fetcher], Duration::from_secs(1));
        let address = Address::with_last_byte(1);

        identifier.prefetcher(tokio::runtime::Handle::current()).prefetch([address]);
        tokio::time::timeout(Duration::from_secs(1), identifier.cache.settled(&[address]))
            .await
            .unwrap();
        let budget = identifier.remaining_budget();

        let identities = identifier.identify_addresses(&[&node(address)]);

        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].label.as_deref(), Some("Prefetched"));
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(identifier.remaining_budget(), budget);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn identify_waits_for_preferred_fetcher_after_partial_result() {
        let sourcify_calls = Arc::new(AtomicUsize::new(0));
        let etherscan_calls = Arc::new(AtomicUsize::new(0));
        let fetchers: Vec<Arc<dyn ExternalFetcherT>> = vec![
            Arc::new(TestFetcher {
                kind: FetcherKind::Sourcify,
                delay: Some(Duration::ZERO),
                contract_name: Some("SourcifyResult"),
                calls: Arc::clone(&sourcify_calls),
                invalid: AtomicBool::new(false),
            }),
            Arc::new(TestFetcher {
                kind: FetcherKind::Etherscan,
                delay: Some(Duration::from_millis(50)),
                contract_name: Some("EtherscanResult"),
                calls: Arc::clone(&etherscan_calls),
                invalid: AtomicBool::new(false),
            }),
        ];
        let mut identifier = test_identifier(fetchers, Duration::from_secs(1));
        let address = Address::with_last_byte(1);

        identifier.prefetcher(tokio::runtime::Handle::current()).prefetch([address]);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if identifier.cache.state().contracts.contains_key(&address) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(identifier.cache.state().in_flight.contains(&address));

        let identities = identifier.identify_addresses(&[&node(address)]);

        assert_eq!(identities[0].label.as_deref(), Some("EtherscanResult"));
        assert_eq!(sourcify_calls.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(etherscan_calls.load(AtomicOrdering::Relaxed), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prefetch_concurrency_is_shared_across_batches() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher: Arc<dyn ExternalFetcherT> = Arc::new(ConcurrentFetcher {
            active: Arc::clone(&active),
            max_active: Arc::clone(&max_active),
            calls: Arc::clone(&calls),
            invalid: AtomicBool::new(false),
        });
        let identifier = test_identifier(vec![fetcher], Duration::from_secs(1));
        let prefetcher = identifier.prefetcher(tokio::runtime::Handle::current());
        let addresses = (1..=6).map(Address::with_last_byte).collect::<Vec<_>>();

        for &address in &addresses {
            prefetcher.prefetch([address]);
        }
        tokio::time::timeout(Duration::from_secs(1), identifier.cache.settled(&addresses))
            .await
            .unwrap();

        assert_eq!(calls.load(AtomicOrdering::Relaxed), addresses.len());
        assert_eq!(max_active.load(AtomicOrdering::Relaxed), 2);
        assert_eq!(active.load(AtomicOrdering::Relaxed), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rate_limit_backoff_is_shared_across_batches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let first_response = Arc::new(Notify::new());
        let fetcher: Arc<dyn ExternalFetcherT> = Arc::new(OnceRateLimitedFetcher {
            calls: Arc::clone(&calls),
            first_response: Arc::clone(&first_response),
            invalid: AtomicBool::new(false),
        });
        let identifier = test_identifier(vec![fetcher], Duration::from_secs(1));
        let prefetcher = identifier.prefetcher(tokio::runtime::Handle::current());
        let addresses = [Address::with_last_byte(1), Address::with_last_byte(2)];

        prefetcher.prefetch([addresses[0]]);
        tokio::time::timeout(Duration::from_secs(1), async {
            while calls.load(AtomicOrdering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        prefetcher.prefetch([addresses[1]]);
        first_response.notify_one();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 1);

        tokio::time::timeout(Duration::from_secs(1), identifier.cache.settled(&addresses))
            .await
            .unwrap();
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn identify_retries_an_abandoned_prefetch_with_remaining_budget() {
        let sourcify_calls = Arc::new(AtomicUsize::new(0));
        let etherscan_calls = Arc::new(AtomicUsize::new(0));
        let fetchers: Vec<Arc<dyn ExternalFetcherT>> = vec![
            Arc::new(TestFetcher {
                kind: FetcherKind::Sourcify,
                delay: Some(Duration::ZERO),
                contract_name: Some("Partial"),
                calls: Arc::clone(&sourcify_calls),
                invalid: AtomicBool::new(false),
            }),
            Arc::new(AbandonedFetcher {
                kind: FetcherKind::Etherscan,
                calls: Arc::clone(&etherscan_calls),
                invalid: AtomicBool::new(false),
            }),
        ];
        let mut identifier = test_identifier(fetchers, Duration::from_secs(1));
        let prefetcher = ExternalPrefetcher {
            fetchers: Arc::clone(&identifier.fetchers),
            cache: Arc::clone(&identifier.cache),
            timeout: Duration::from_millis(20),
            handle: tokio::runtime::Handle::current(),
        };
        let address = Address::with_last_byte(1);

        prefetcher.prefetch([address]);
        tokio::time::timeout(Duration::from_secs(1), async {
            while etherscan_calls.load(AtomicOrdering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let identities = identifier.identify_addresses(&[&node(address)]);

        assert_eq!(identities[0].label.as_deref(), Some("Retried"));
        assert_eq!(sourcify_calls.load(AtomicOrdering::Relaxed), 2);
        assert_eq!(etherscan_calls.load(AtomicOrdering::Relaxed), 2);
        assert!(!identifier.remaining_budget().is_zero());
    }

    #[test]
    fn dropping_runtime_releases_claim_before_prefetch_first_poll() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher = test_fetcher(Some(Duration::ZERO), Some("NeverPolled"), &calls);
        let identifier = test_identifier(vec![fetcher], Duration::from_secs(1));
        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let address = Address::with_last_byte(1);

        identifier.prefetcher(runtime.handle().clone()).prefetch([address]);
        assert!(identifier.cache.state().in_flight.contains(&address));
        drop(runtime);

        assert!(identifier.cache.state().in_flight.is_empty());
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn claim_skips_addresses_cached_after_the_initial_lookup() {
        let identifier = test_identifier(Vec::new(), Duration::from_secs(1));
        let address = Address::with_last_byte(1);
        identifier.cache.record(address, (FetcherKind::Sourcify, Some(metadata("Cached"))), true);

        let (claim, to_await) = identifier.cache.claim(&[address]);

        assert!(claim.addresses.is_empty());
        assert!(to_await.is_empty());
        assert!(identifier.cache.state().in_flight.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prefetch_skips_cached_and_in_flight_addresses() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher = test_fetcher(Some(Duration::from_millis(10)), Some("Once"), &calls);
        let identifier = test_identifier(vec![fetcher], Duration::from_secs(1));
        let prefetcher = identifier.prefetcher(tokio::runtime::Handle::current());
        let address = Address::with_last_byte(1);

        prefetcher.prefetch([address, address]);
        prefetcher.prefetch([address]);
        tokio::time::timeout(Duration::from_secs(1), identifier.cache.settled(&[address]))
            .await
            .unwrap();
        prefetcher.prefetch([address]);

        assert_eq!(calls.load(AtomicOrdering::Relaxed), 1);
        assert!(identifier.cache.state().in_flight.is_empty());
    }

    #[tokio::test]
    async fn prefetch_is_disabled_once_budget_is_spent() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher = test_fetcher(Some(Duration::ZERO), Some("Never"), &calls);
        let identifier = test_identifier(vec![fetcher], Duration::ZERO);
        let address = Address::with_last_byte(1);

        identifier.prefetcher(tokio::runtime::Handle::current()).prefetch([address]);
        tokio::task::yield_now().await;

        assert_eq!(calls.load(AtomicOrdering::Relaxed), 0);
        assert!(identifier.cache.state().in_flight.is_empty());
        assert!(!identifier.cache.state().contracts.contains_key(&address));
    }

    #[tokio::test]
    async fn prefetch_timeout_releases_in_flight_addresses_without_spending_budget() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetcher = test_fetcher(None, None, &calls);
        let identifier = test_identifier(vec![fetcher], Duration::from_millis(20));
        let address = Address::with_last_byte(1);

        identifier.prefetcher(tokio::runtime::Handle::current()).prefetch([address]);
        tokio::time::timeout(Duration::from_secs(1), identifier.cache.settled(&[address]))
            .await
            .unwrap();

        assert_eq!(calls.load(AtomicOrdering::Relaxed), 1);
        assert!(identifier.cache.state().in_flight.is_empty());
        assert!(!identifier.cache.state().contracts.contains_key(&address));
        assert_eq!(identifier.remaining_budget(), Duration::from_millis(20));
    }
}
