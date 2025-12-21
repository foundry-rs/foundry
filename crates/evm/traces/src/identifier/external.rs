use super::{IdentifiedAddress, TraceIdentifier};
use crate::debug::ContractSources;
use alloy_primitives::{
    Address,
    map::{Entry, HashMap},
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
}

impl ExternalIdentifier {
    /// Creates a new external identifier with the given client
    pub fn new(config: &Config, mut chain: Option<Chain>) -> eyre::Result<Option<Self>> {
        if config.offline {
            return Ok(None);
        }

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
            fetchers.push(Arc::new(EtherscanFetcher::new(config.into_client()?)));
        }
        if fetchers.is_empty() {
            debug!(target: "evm::traces::external", "no fetchers enabled");
            return Ok(None);
        }

        Ok(Some(Self { fetchers, contracts: Default::default() }))
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
                    eyre::bail!("{output}")
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
            artifact_id: None,
        }
    }
}

impl TraceIdentifier for ExternalIdentifier {
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        if nodes.is_empty() {
            return Vec::new();
        }

        trace!(target: "evm::traces::external", "identify {} addresses", nodes.len());

        let mut identities = Vec::new();
        let mut to_fetch = Vec::new();

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
                to_fetch.push(address);
            }
        }

        if to_fetch.is_empty() {
            return identities;
        }
        trace!(target: "evm::traces::external", "fetching {} addresses", to_fetch.len());

        let fetchers =
            self.fetchers.iter().map(|fetcher| ExternalFetcher::new(fetcher.clone(), &to_fetch));
        let fetched_identities = foundry_common::block_on(
            futures::stream::select_all(fetchers)
                .filter_map(|(address, value)| {
                    let addr = value
                        .1
                        .as_ref()
                        .map(|metadata| self.identify_from_metadata(address, metadata));
                    match self.contracts.entry(address) {
                        Entry::Occupied(mut occupied_entry) => {
                            // Override if:
                            // - new is from Etherscan and old is not
                            // - new is Some and old is None, meaning verified only in one source
                            if !matches!(occupied_entry.get().0, FetcherKind::Etherscan)
                                || value.1.is_none()
                            {
                                occupied_entry.insert(value);
                            }
                        }
                        Entry::Vacant(vacant_entry) => {
                            vacant_entry.insert(value);
                        }
                    }
                    async move { addr }
                })
                .collect::<Vec<IdentifiedAddress<'_>>>(),
        );
        trace!(target: "evm::traces::external", "fetched {} addresses: {fetched_identities:#?}", fetched_identities.len());

        identities.extend(fetched_identities);
        identities
    }
}

type FetchFuture =
    Pin<Box<dyn Future<Output = (Address, Result<Option<Metadata>, EtherscanError>)>>>;

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
                            pin.backoff = Some(tokio::time::interval(pin.timeout));
                            pin.queue.push(addr);
                        }
                        Err(EtherscanError::InvalidApiKey) => {
                            warn!(target: "evm::traces::external", "invalid api key");
                            // mark key as invalid
                            pin.fetcher.invalid_api_key().store(true, Ordering::Relaxed);
                            return Poll::Ready(None);
                        }
                        Err(EtherscanError::BlockedByCloudflare) => {
                            warn!(target: "evm::traces::external", "blocked by cloudflare");
                            // mark key as invalid
                            pin.fetcher.invalid_api_key().store(true, Ordering::Relaxed);
                            return Poll::Ready(None);
                        }
                        Err(err) => {
                            warn!(target: "evm::traces::external", ?err, "could not get info");
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
    fn new(client: foundry_block_explorers::Client) -> Self {
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
            client: reqwest::Client::new(),
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
        let response = self.client.get(url).send().await?;
        let code = response.status();
        let response: SourcifyResponse = response.json().await?;
        trace!(target: "evm::traces::external", "Sourcify response for {address}: {response:#?}");
        match code.as_u16() {
            // Not verified.
            404 => return Err(EtherscanError::ContractCodeNotVerified(address)),
            // Too many requests.
            429 => return Err(EtherscanError::RateLimitExceeded),
            _ => {}
        }
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
