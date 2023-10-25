use super::{AddressIdentity, TraceIdentifier};
use crate::utils::RuntimeOrHandle;
use alloy_primitives::Address;
use foundry_block_explorers::{
    contract::{ContractMetadata, Metadata},
    errors::EtherscanError,
};
use foundry_common::compile::{self, ContractSources};
use foundry_config::{Chain, Config};
use futures::{
    future::{join_all, Future},
    stream::{FuturesUnordered, Stream, StreamExt},
    task::{Context, Poll},
    TryFutureExt,
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
#[derive(Default)]
pub struct EtherscanIdentifier {
    /// The Etherscan client
    client: Option<Arc<foundry_block_explorers::Client>>,
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
    pub fn new(config: &Config, chain: Option<impl Into<Chain>>) -> eyre::Result<Self> {
        if let Some(config) = config.get_etherscan_config_with_chain(chain)? {
            trace!(target: "etherscanidentifier", chain=?config.chain, url=?config.api_url, "using etherscan identifier");
            Ok(Self {
                client: Some(Arc::new(config.into_client()?)),
                invalid_api_key: Arc::new(Default::default()),
                contracts: BTreeMap::new(),
                sources: BTreeMap::new(),
            })
        } else {
            Ok(Default::default())
        }
    }

    /// Goes over the list of contracts we have pulled from the traces, clones their source from
    /// Etherscan and compiles them locally, for usage in the debugger.
    pub async fn get_compiled_contracts(&self) -> eyre::Result<ContractSources> {
        // TODO: Add caching so we dont double-fetch contracts.
        let contracts_iter = self
            .contracts
            .iter()
            // filter out vyper files
            .filter(|(_, metadata)| !metadata.is_vyper());

        let outputs_fut = contracts_iter
            .clone()
            .map(|(address, metadata)| {
                println!("Compiling: {} {address:?}", metadata.contract_name);
                let err_msg = format!(
                    "Failed to compile contract {} from {address:?}",
                    metadata.contract_name
                );
                compile::compile_from_source(metadata).map_err(move |err| err.wrap_err(err_msg))
            })
            .collect::<Vec<_>>();

        // poll all the futures concurrently
        let artifacts = join_all(outputs_fut).await;

        let mut sources: ContractSources = Default::default();

        // construct the map
        for (results, (_, metadata)) in artifacts.into_iter().zip(contracts_iter) {
            // get the inner type
            let (artifact_id, file_id, bytecode) = results?;
            sources
                .0
                .entry(artifact_id.clone().name)
                .or_default()
                .insert(file_id, (metadata.source_code(), bytecode));
        }

        Ok(sources)
    }
}

impl TraceIdentifier for EtherscanIdentifier {
    fn identify_addresses<'a, A>(&mut self, addresses: A) -> Vec<AddressIdentity>
    where
        A: Iterator<Item = (&'a Address, Option<&'a [u8]>)>,
    {
        trace!(target: "etherscanidentifier", "identify {:?} addresses", addresses.size_hint().1);

        let Some(client) = self.client.clone() else {
            // no client was configured
            return Vec::new()
        };

        if self.invalid_api_key.load(Ordering::Relaxed) {
            // api key was marked as invalid
            return Vec::new()
        }

        let mut fetcher = EtherscanFetcher::new(
            client,
            Duration::from_secs(1),
            5,
            Arc::clone(&self.invalid_api_key),
        );

        for (addr, _) in addresses {
            if !self.contracts.contains_key(addr) {
                fetcher.push(*addr);
            }
        }

        let fut = fetcher
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
            .collect();

        RuntimeOrHandle::new().block_on(fut)
    }
}

type EtherscanFuture =
    Pin<Box<dyn Future<Output = (Address, Result<ContractMetadata, EtherscanError>)>>>;

/// A rate limit aware Etherscan client.
///
/// Fetches information about multiple addresses concurrently, while respecting rate limits.
pub struct EtherscanFetcher {
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
    pub fn new(
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

    pub fn push(&mut self, address: Address) {
        self.queue.push(address);
    }

    fn queue_next_reqs(&mut self) {
        while self.in_progress.len() < self.concurrency {
            if let Some(addr) = self.queue.pop() {
                let client = Arc::clone(&self.client);
                trace!(target: "etherscanidentifier", "fetching info for {:?}", addr);
                self.in_progress.push(Box::pin(async move {
                    let res = client.contract_source_code(addr).await;
                    (addr, res)
                }));
            } else {
                break
            }
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
                            warn!(target: "etherscanidentifier", "rate limit exceeded on attempt");
                            pin.backoff = Some(tokio::time::interval(pin.timeout));
                            pin.queue.push(addr);
                        }
                        Err(EtherscanError::InvalidApiKey) => {
                            warn!(target: "etherscanidentifier", "invalid api key");
                            // mark key as invalid
                            pin.invalid_api_key.store(true, Ordering::Relaxed);
                            return Poll::Ready(None)
                        }
                        Err(EtherscanError::BlockedByCloudflare) => {
                            warn!(target: "etherscanidentifier", "blocked by cloudflare");
                            // mark key as invalid
                            pin.invalid_api_key.store(true, Ordering::Relaxed);
                            return Poll::Ready(None)
                        }
                        Err(err) => {
                            warn!(target: "etherscanidentifier", "could not get etherscan info: {:?}", err);
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
