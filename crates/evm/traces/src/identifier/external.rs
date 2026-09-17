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
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::time::{Duration, Interval};

/// A trace identifier that tries to identify addresses using Etherscan.
pub struct ExternalIdentifier {
    fetchers: Vec<Arc<dyn ExternalFetcherT>>,
    /// Cached contracts.
    contracts: HashMap<Address, (FetcherKind, Option<Metadata>)>,
    /// Remaining time external identification may block trace rendering.
    remaining_budget: Duration,
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

        Ok(Some(Self {
            fetchers,
            contracts: Default::default(),
            remaining_budget: Duration::from_secs(timeout),
        }))
    }

    /// Goes over the list of contracts we have pulled from the traces, clones their source from
    /// Etherscan and compiles them locally, for usage in the debugger.
    pub async fn get_compiled_contracts(&self) -> eyre::Result<ContractSources> {
        // Collect contract info upfront so we can reference it in error messages
        let contracts_info: Vec<_> = self
            .contracts
            .iter()
            // filter out vyper files and contracts without metadata
            .filter_map(|(addr, (_, metadata))| {
                if let Some(metadata) = metadata.as_ref()
                    && !metadata.is_vyper()
                {
                    Some((*addr, metadata))
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
            constructor_args_offset: None,
            artifact_id: None,
        }
    }

    fn cache_fetched(&mut self, address: Address, value: (FetcherKind, Option<Metadata>)) {
        match self.contracts.entry(address) {
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
    }

    async fn fetch_addresses_async(&mut self, addresses: &[Address]) {
        if addresses.is_empty() || self.remaining_budget.is_zero() {
            return;
        }

        let fetchers = self
            .fetchers
            .clone()
            .into_iter()
            .map(|fetcher| ExternalFetcher::new(fetcher, addresses));
        let started = tokio::time::Instant::now();
        let timed_out = tokio::time::timeout(self.remaining_budget, async {
            let mut fetched = futures::stream::select_all(fetchers);
            while let Some((address, value)) = fetched.next().await {
                self.cache_fetched(address, value);
            }
        })
        .await
        .is_err();
        self.remaining_budget = self.remaining_budget.saturating_sub(started.elapsed());
        if timed_out {
            self.remaining_budget = Duration::ZERO;
            warn!(target: "evm::traces::external", "external identification timed out; disabling it for the remainder of this session");
        }
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
            let to_fetch = chains
                .iter()
                .filter_map(|chain| chain.current)
                .filter(|address| !self.contracts.contains_key(address))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            self.fetch_addresses_async(&to_fetch).await;

            let mut has_next = false;
            for chain in &mut chains {
                let Some(current) = chain.current else { continue };
                if !chain.visited.insert(current) {
                    chain.current = None;
                    chain.complete = false;
                    continue;
                }
                let Some((_, Some(metadata))) = self.contracts.get(&current) else {
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

        let mut identities = Vec::new();
        let mut to_fetch = AddressSet::default();

        // Check cache first.
        for &node in nodes {
            let address = node.trace.address;
            if let Some((_, metadata)) = self.contracts.get(&address) {
                if let Some(metadata) = metadata {
                    identities.push(self.identify_from_metadata(address, metadata));
                } else {
                    // Do nothing. We know that this contract was not verified.
                }
            } else {
                to_fetch.insert(address);
            }
        }

        if to_fetch.is_empty() {
            return identities;
        }
        if self.remaining_budget.is_zero() {
            return identities;
        }
        trace!(target: "evm::traces::external", "fetching {} addresses", to_fetch.len());

        let to_fetch = to_fetch.into_iter().collect::<Vec<_>>();
        foundry_common::block_on(self.fetch_addresses_async(&to_fetch));

        for address in to_fetch {
            if let Some((_, Some(metadata))) = self.contracts.get(&address) {
                identities.push(self.identify_from_metadata(address, metadata));
            }
        }
        trace!(target: "evm::traces::external", "identified {} addresses", identities.len());
        identities
    }
}

type FetchFuture =
    Pin<Box<dyn Future<Output = (Address, Result<Option<Metadata>, EtherscanError>)>>>;

/// Maximum number of times a single address is retried through a transient Cloudflare
/// block before we give up on it. Bounded so a persistent block can't loop forever.
const MAX_CLOUDFLARE_RETRIES: u32 = 5;

fn backoff_interval(period: Duration) -> Interval {
    tokio::time::interval_at(tokio::time::Instant::now() + period, period)
}

/// A rate limit aware fetcher.
///
/// Fetches information about multiple addresses concurrently, while respecting rate limits.
struct ExternalFetcher {
    /// The fetcher
    fetcher: Arc<dyn ExternalFetcherT>,
    /// The time we wait if we hit the rate limit
    timeout: Duration,
    /// The interval we are currently waiting for before making a new request
    backoff: Option<Interval>,
    /// The maximum amount of requests to send concurrently
    concurrency: usize,
    /// The addresses we have yet to make requests for
    queue: Vec<Address>,
    /// The in progress requests
    in_progress: FuturesUnordered<FetchFuture>,
    /// Per-address retry counter for transient Cloudflare blocks.
    attempts: HashMap<Address, u32>,
}

impl ExternalFetcher {
    fn new(fetcher: Arc<dyn ExternalFetcherT>, to_fetch: &[Address]) -> Self {
        Self {
            timeout: fetcher.timeout(),
            backoff: None,
            concurrency: fetcher.concurrency(),
            fetcher,
            queue: to_fetch.to_vec(),
            in_progress: FuturesUnordered::new(),
            attempts: HashMap::default(),
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

        let _guard =
            info_span!("evm::traces::external", kind=?pin.fetcher.kind(), "ExternalFetcher")
                .entered();

        if pin.fetcher.invalid_api_key().load(Ordering::Relaxed) {
            return Poll::Ready(None);
        }

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
                        Ok(metadata) => {
                            return Poll::Ready(Some((addr, (pin.fetcher.kind(), metadata))));
                        }
                        Err(EtherscanError::ContractCodeNotVerified(_)) => {
                            return Poll::Ready(Some((addr, (pin.fetcher.kind(), None))));
                        }
                        Err(EtherscanError::RateLimitExceeded) => {
                            warn!(target: "evm::traces::external", "rate limit exceeded on attempt");
                            pin.backoff = Some(backoff_interval(pin.timeout));
                            pin.queue.push(addr);
                        }
                        Err(EtherscanError::InvalidApiKey) => {
                            warn!(target: "evm::traces::external", "invalid api key");
                            // mark key as invalid
                            pin.fetcher.invalid_api_key().store(true, Ordering::Relaxed);
                            return Poll::Ready(None);
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
                                pin.backoff = Some(backoff_interval(pin.timeout));
                                pin.queue.push(addr);
                            } else {
                                warn!(target: "evm::traces::external", "blocked by cloudflare, giving up on address");
                                return Poll::Ready(Some((addr, (pin.fetcher.kind(), None))));
                            }
                        }
                        Err(err) => {
                            warn!(target: "evm::traces::external", ?err, "could not get info");
                            // Cache the failure so we don't re-fetch on subsequent arenas.
                            return Poll::Ready(Some((addr, (pin.fetcher.kind(), None))));
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
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering as AtomicOrdering},
        },
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
        ExternalIdentifier { fetchers, contracts: Default::default(), remaining_budget }
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

        let collected: Vec<_> = ExternalFetcher::new(fetcher, &addrs).collect().await;

        let got: StdHashSet<Address> = collected.into_iter().map(|(addr, _)| addr).collect();
        let want: StdHashSet<Address> = addrs.into_iter().collect();
        assert_eq!(got, want, "every address must be yielded despite a transient cloudflare block");
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

        assert!(identifier.remaining_budget.is_zero());
        assert_eq!(
            identifier.contracts[&address].1.as_ref().unwrap().contract_name,
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
        assert!(identifier.contracts[&first].1.is_some());
        assert!(identifier.remaining_budget < Duration::from_millis(15));

        identifier.fetch_addresses_async(&[second]).await;
        assert!(identifier.remaining_budget.is_zero());
        assert!(!identifier.contracts.contains_key(&second));
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

        assert!(identifier.remaining_budget.is_zero());
        assert!(calls.load(AtomicOrdering::Relaxed) > 1);
    }

    #[test]
    fn etherscan_metadata_takes_precedence() {
        let address = Address::with_last_byte(1);
        let mut identifier = test_identifier(Vec::new(), Duration::ZERO);

        identifier
            .cache_fetched(address, (FetcherKind::Sourcify, Some(metadata("SourcifyResult"))));
        identifier.cache_fetched(address, (FetcherKind::Etherscan, None));
        assert_eq!(
            identifier.contracts[&address].1.as_ref().unwrap().contract_name,
            "SourcifyResult"
        );

        identifier
            .cache_fetched(address, (FetcherKind::Etherscan, Some(metadata("EtherscanResult"))));
        assert_eq!(
            identifier.contracts[&address].1.as_ref().unwrap().contract_name,
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
        let identity = identifier.identify_from_metadata(proxy, &proxy_metadata);
        assert_eq!(identity.contract.as_deref(), Some("Proxy"));
        identifier.cache_fetched(proxy, (FetcherKind::Etherscan, Some(proxy_metadata)));
        identifier
            .cache_fetched(implementation_address, (FetcherKind::Etherscan, Some(implementation)));

        let mut results = identifier.get_abis(&[proxy]).await;
        let (result_address, result) = results.pop().unwrap();
        let (abis, complete) = result.unwrap();
        let event_names =
            abis.into_iter().map(|abi| abi.events.into_keys().next().unwrap()).collect::<Vec<_>>();

        assert_eq!(result_address, proxy);
        assert!(complete);
        assert_eq!(event_names, ["ImplementationEvent", "ProxyEvent"]);

        identifier.contracts.remove(&implementation_address);
        let (_, result) = identifier.get_abis(&[proxy]).await.pop().unwrap();
        let (abis, complete) = result.unwrap();
        assert_eq!(abis.len(), 1);
        assert!(!complete);
    }
}
