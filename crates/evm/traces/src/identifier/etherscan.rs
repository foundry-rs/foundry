use super::{AddressIdentity, TraceIdentifier};
use crate::debug::ContractSources;
use alloy_primitives::Address;
use foundry_block_explorers::{
    contract::{ContractMetadata, Metadata},
    errors::EtherscanError,
};
use foundry_common::compile::etherscan_project;
use foundry_config::{Chain, Config};
use futures::{
    future::{join_all, Future},
    stream::{FuturesUnordered, Stream, StreamExt},
    task::{Context, Poll},
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::time::{Duration, Interval};

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
}

impl EtherscanIdentifier {
    /// Creates a new Etherscan identifier with the given client
    pub fn new(config: &Config, chain: Option<Chain>) -> eyre::Result<Option<Self>> {
        // In offline mode, don't use Etherscan.
        if config.offline {
            return Ok(None);
        }
        let Some(config) = config.get_etherscan_config_with_chain(chain)? else {
            return Ok(None);
        };
        trace!(target: "traces::etherscan", chain=?config.chain, url=?config.api_url, "using etherscan identifier");
        Ok(Some(Self {
            client: Arc::new(config.into_client()?),
            invalid_api_key: Arc::new(AtomicBool::new(false)),
            contracts: BTreeMap::new(),
            sources: BTreeMap::new(),
        }))
    }

    /// Goes over the list of contracts we have pulled from the traces, clones their source from
    /// Etherscan and compiles them locally, for usage in the debugger.
    pub async fn get_compiled_contracts(&self) -> eyre::Result<ContractSources> {
        // TODO: Add caching so we dont double-fetch contracts.
        let outputs_fut = self
            .contracts
            .iter()
            // filter out vyper files
            .filter(|(_, metadata)| !metadata.is_vyper())
            .map(|(address, metadata)| async move {
                println!("Compiling: {} {address}", metadata.contract_name);
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
        for res in outputs {
            let (project, output, _root) = res?;
            sources.insert(&output, project.root(), None)?;
        }

        Ok(sources)
    }
}

impl TraceIdentifier for EtherscanIdentifier {
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity<'_>>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        trace!(target: "evm::traces", "identify {:?} addresses", addresses.size_hint().1);

        if self.invalid_api_key.load(Ordering::Relaxed) {
            // api key was marked as invalid
            return Vec::new()
        }

        let mut identities = Vec::new();
        let mut fetcher = EtherscanFetcher::new(
            self.client.clone(),
            Duration::from_secs(1),
            5,
            Arc::clone(&self.invalid_api_key),
        );

        for (addr, _) in addresses {
            if let Some(metadata) = self.contracts.get(addr) {
                let label = metadata.contract_name.clone();
                let abi = metadata.abi().ok().map(Cow::Owned);

                identities.push(AddressIdentity {
                    address: *addr,
                    label: Some(label.clone()),
                    contract: Some(label),
                    abi,
                    artifact_id: None,
                });
            } else {
                fetcher.push(*addr);
            }
        }

        let fetched_identities = foundry_common::block_on(
            fetcher
                .map(|(address, metadata)| {
                    let label = metadata.contract_name.clone();
                    let abi = metadata.abi().ok().map(Cow::Owned);
                    self.contracts.insert(address, metadata);

                    AddressIdentity {
                        address,
                        label: Some(label.clone()),
                        contract: Some(label),
                        abi,
                        artifact_id: None,
                    }
                })
                .collect::<Vec<AddressIdentity<'_>>>(),
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
            if let Some(mut backoff) = pin.backoff.take() {
                if backoff.poll_tick(cx).is_pending() {
                    pin.backoff = Some(backoff);
                    return Poll::Pending
                }
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
                                return Poll::Ready(Some((addr, item)))
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
                            return Poll::Ready(None)
                        }
                        Err(EtherscanError::BlockedByCloudflare) => {
                            warn!(target: "traces::etherscan", "blocked by cloudflare");
                            // mark key as invalid
                            pin.invalid_api_key.store(true, Ordering::Relaxed);
                            return Poll::Ready(None)
                        }
                        Err(err) => {
                            warn!(target: "traces::etherscan", "could not get etherscan info: {:?}", err);
                        }
                    }
                }
            }

            if !made_progress_this_iter {
                return Poll::Pending
            }
        }
    }
}
