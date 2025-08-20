use super::{IdentifiedAddress, TraceIdentifier};
use crate::debug::ContractSources;
use alloy_primitives::Address;
use foundry_block_explorers::{
    contract::{ContractMetadata, Metadata},
    errors::EtherscanError,
};
use foundry_common::compile::etherscan_project;
use foundry_config::{Chain, Config};
use futures::{
    future::join_all,
    stream::{FuturesUnordered, Stream, StreamExt},
    task::{Context, Poll},
};
use revm_inspectors::tracing::types::CallTraceNode;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::time::{Duration, Interval};

/// Cache entry for compiled contracts
#[derive(Clone, Debug)]
struct CachedContract {
    /// The compiled contract sources
    sources: ContractSources,
    /// Timestamp when this entry was cached
    cached_at: Instant,
}

/// A trace identifier that tries to identify addresses using Etherscan.
pub struct EtherscanIdentifier {
    /// The Etherscan client
    client: Arc<foundry_block_explorers::Client>,
    /// Tracks whether the API key provides was marked as invalid
    ///
    /// After the first [EtherscanError::InvalidApiKey] this will get set to true, so we can
    /// prevent any further attempts
    invalid_api_key: Arc<AtomicBool>,
    pub contracts: BTreeMap<Address, Metadata>,
    pub sources: BTreeMap<u32, String>,
    /// Cache for compiled contracts to avoid re-fetching and re-compiling
    compiled_cache: Arc<Mutex<HashMap<Address, CachedContract>>>,
}

impl EtherscanIdentifier {
    /// Creates a new Etherscan identifier with the given client
    pub fn new(config: &Config, chain: Option<Chain>) -> eyre::Result<Option<Self>> {
        // In offline mode, don't use Etherscan.
        if config.offline {
            return Ok(None);
        }

        let config = match config.get_etherscan_config_with_chain(chain) {
            Ok(Some(config)) => config,
            Ok(None) => {
                warn!(target: "traces::etherscan", "etherscan config not found");
                return Ok(None);
            }
            Err(err) => {
                warn!(?err, "failed to get etherscan config");
                return Ok(None);
            }
        };

        trace!(target: "traces::etherscan", chain=?config.chain, url=?config.api_url, "using etherscan identifier");
        Ok(Some(Self {
            client: Arc::new(config.into_client()?),
            invalid_api_key: Arc::new(AtomicBool::new(false)),
            contracts: BTreeMap::new(),
            sources: BTreeMap::new(),
            compiled_cache: Arc::new(Mutex::new(HashMap::new())),
        }))
    }

    /// Goes over the list of contracts we have pulled from the traces, clones their source from
    /// Etherscan and compiles them locally, for usage in the debugger.
    ///
    /// This implementation includes caching to avoid re-fetching and re-compiling contracts.
    pub async fn get_compiled_contracts(&self) -> eyre::Result<ContractSources> {
        // Cache expiry time (1 hour)
        const CACHE_DURATION_SECS: u64 = 3600;
        
        let mut sources: ContractSources = Default::default();
        let mut to_compile = Vec::new();
        
        // First pass: check cache and collect contracts that need compilation
        {
            let mut cache = self.compiled_cache.lock().unwrap();
            let now = Instant::now();
            
            // Clean up expired entries
            cache.retain(|_, entry| {
                now.duration_since(entry.cached_at).as_secs() < CACHE_DURATION_SECS
            });
            
            for (address, metadata) in &self.contracts {
                // Skip vyper files
                if metadata.is_vyper() {
                    continue;
                }
                
                // Check if we have a valid cached version
                if let Some(cached) = cache.get(address) {
                    // Merge cached sources into result
                    for (build_id, source_map) in &cached.sources.sources_by_id {
                        sources.sources_by_id
                            .entry(build_id.clone())
                            .or_insert_with(HashMap::new)
                            .extend(source_map.clone());
                    }
                    for (name, artifacts) in &cached.sources.artifacts_by_name {
                        sources.artifacts_by_name
                            .entry(name.clone())
                            .or_insert_with(Vec::new)
                            .extend(artifacts.clone());
                    }
                    trace!(target: "traces::etherscan", ?address, "using cached compiled contract");
                } else {
                    to_compile.push((*address, metadata));
                }
            }
        }
        
        // Second pass: compile contracts that weren't in cache
        if !to_compile.is_empty() {
            let outputs_fut = to_compile
                .iter()
                .map(|(address, metadata)| async move {
                    sh_println!("Compiling: {} {address}", metadata.contract_name)?;
                    let root = tempfile::tempdir()?;
                    let root_path = root.path();
                    let project = etherscan_project(metadata, root_path)?;
                    let output = project.compile()?;

                    if output.has_compiler_errors() {
                        eyre::bail!("{output}")
                    }

                    Ok((*address, project, output, root))
                })
                .collect::<Vec<_>>();

            // Poll all the futures concurrently
            let outputs = join_all(outputs_fut).await;

            // Process compilation results and update cache
            let mut cache = self.compiled_cache.lock().unwrap();
            let now = Instant::now();
            
            for res in outputs {
                let (address, project, output, _root) = res?;
                
                // Create a temporary ContractSources for this contract
                let mut contract_sources = ContractSources::default();
                contract_sources.insert(&output, project.root(), None)?;
                
                // Cache the compiled contract
                cache.insert(address, CachedContract {
                    sources: contract_sources.clone(),
                    cached_at: now,
                });
                
                // Merge into final result
                for (build_id, source_map) in contract_sources.sources_by_id {
                    sources.sources_by_id
                        .entry(build_id)
                        .or_insert_with(HashMap::new)
                        .extend(source_map);
                }
                for (name, artifacts) in contract_sources.artifacts_by_name {
                    sources.artifacts_by_name
                        .entry(name)
                        .or_insert_with(Vec::new)
                        .extend(artifacts);
                }
                
                trace!(target: "traces::etherscan", ?address, "compiled and cached contract");
            }
        }

        Ok(sources)
    }
    
    /// Clear the compilation cache
    pub fn clear_cache(&self) {
        self.compiled_cache.lock().unwrap().clear();
        trace!(target: "traces::etherscan", "cleared compilation cache");
    }

    fn identify_from_metadata(
        &self,
        address: Address,
        metadata: &Metadata,
    ) -> IdentifiedAddress<'static> {
        let label = metadata.contract_name.clone();
        let abi = metadata.abi().ok().map(Cow::Owned);
        IdentifiedAddress {
            address,
            label: Some(label.clone()),
            contract: Some(label),
            abi,
            artifact_id: None,
        }
    }
}

impl TraceIdentifier for EtherscanIdentifier {
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        if self.invalid_api_key.load(Ordering::Relaxed) || nodes.is_empty() {
            return Vec::new();
        }

        trace!(target: "evm::traces::etherscan", "identify {} addresses", nodes.len());

        let mut identities = Vec::new();
        let mut fetcher = EtherscanFetcher::new(
            self.client.clone(),
            Duration::from_secs(1),
            5,
            Arc::clone(&self.invalid_api_key),
        );

        for &node in nodes {
            let address = node.trace.address;
            if let Some(metadata) = self.contracts.get(&address) {
                identities.push(self.identify_from_metadata(address, metadata));
            } else {
                fetcher.push(address);
            }
        }

        let fetched_identities = foundry_common::block_on(
            fetcher
                .map(|(address, metadata)| {
                    let addr = self.identify_from_metadata(address, &metadata);
                    self.contracts.insert(address, metadata);
                    addr
                })
                .collect::<Vec<IdentifiedAddress<'_>>>(),
        );

        identities.extend(fetched_identities);
        identities
    }
}

type EtherscanFuture =
    Pin<Box<dyn Future<Output = (Address, Result<ContractMetadata, EtherscanError>)>>>;

/// A rate limit aware Etherscan client.
///
/// Fetches information about multiple addresses concurrently, while respecting rate limits.
struct EtherscanFetcher {
    /// The Etherscan client
    client: Arc<foundry_block_explorers::Client>,
    /// The time we wait if we hit the rate limit
    timeout: Duration,
    /// The interval we are currently waiting for before making a new request
    backoff: Option<Interval>,
    /// The maximum amount of requests to send concurrently
    concurrency: usize,
    /// The addresses we have yet to make requests for
    queue: Vec<Address>,
    /// The in progress requests
    in_progress: FuturesUnordered<EtherscanFuture>,
    /// tracks whether the API key provides was marked as invalid
    invalid_api_key: Arc<AtomicBool>,
}

impl EtherscanFetcher {
    fn new(
        client: Arc<foundry_block_explorers::Client>,
        timeout: Duration,
        concurrency: usize,
        invalid_api_key: Arc<AtomicBool>,
    ) -> Self {
        Self {
            client,
            timeout,
            backoff: None,
            concurrency,
            queue: Vec::new(),
            in_progress: FuturesUnordered::new(),
            invalid_api_key,
        }
    }

    fn push(&mut self, address: Address) {
        self.queue.push(address);
    }

    fn queue_next_reqs(&mut self) {
        while self.in_progress.len() < self.concurrency {
            let Some(addr) = self.queue.pop() else { break };
            let client = Arc::clone(&self.client);
            self.in_progress.push(Box::pin(async move {
                trace!(target: "traces::etherscan", ?addr, "fetching info");
                let res = client.contract_source_code(addr).await;
                (addr, res)
            }));
        }
    }
}

impl Stream for EtherscanFetcher {
    type Item = (Address, Metadata);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();

        loop {
            if let Some(mut backoff) = pin.backoff.take()
                && backoff.poll_tick(cx).is_pending()
            {
                pin.backoff = Some(backoff);
                return Poll::Pending;
            }

            pin.queue_next_reqs();

            let mut made_progress_this_iter = false;
            match pin.in_progress.poll_next_unpin(cx) {
                Poll::Pending => {}
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some((addr, res))) => {
                    made_progress_this_iter = true;
                    match res {
                        Ok(mut metadata) => {
                            if let Some(item) = metadata.items.pop() {
                                return Poll::Ready(Some((addr, item)));
                            }
                        }
                        Err(EtherscanError::RateLimitExceeded) => {
                            warn!(target: "traces::etherscan", "rate limit exceeded on attempt");
                            pin.backoff = Some(tokio::time::interval(pin.timeout));
                            pin.queue.push(addr);
                        }
                        Err(EtherscanError::InvalidApiKey) => {
                            warn!(target: "traces::etherscan", "invalid api key");
                            // mark key as invalid
                            pin.invalid_api_key.store(true, Ordering::Relaxed);
                            return Poll::Ready(None);
                        }
                        Err(EtherscanError::BlockedByCloudflare) => {
                            warn!(target: "traces::etherscan", "blocked by cloudflare");
                            // mark key as invalid
                            pin.invalid_api_key.store(true, Ordering::Relaxed);
                            return Poll::Ready(None);
                        }
                        Err(err) => {
                            warn!(target: "traces::etherscan", "could not get etherscan info: {:?}", err);
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
