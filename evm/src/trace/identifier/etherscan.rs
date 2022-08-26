use super::{AddressIdentity, TraceIdentifier};
use ethers::{
    abi::{Abi, Address},
    etherscan,
    prelude::{contract::ContractMetadata, errors::EtherscanError},
    solc::utils::RuntimeOrHandle,
};
use foundry_config::{Chain, Config};
use futures::{
    future::Future,
    stream::{FuturesUnordered, Stream, StreamExt},
    task::{Context, Poll},
};
use std::{
    borrow::Cow,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::time::{Duration, Interval};
use tracing::{trace, warn};

/// A trace identifier that tries to identify addresses using Etherscan.
#[derive(Default)]
pub struct EtherscanIdentifier {
    /// The Etherscan client
    client: Option<Arc<etherscan::Client>>,
    /// Tracks whether the API key provides was marked as invalid
    ///
    /// After the first [EtherscanError::InvalidApiKey] this will get set to true, so we can
    /// prevent any further attempts
    invalid_api_key: Arc<AtomicBool>,
}

impl EtherscanIdentifier {
    /// Creates a new Etherscan identifier with the given client
    pub fn new(config: &Config, chain: Option<impl Into<Chain>>) -> eyre::Result<Self> {
        if let Some(config) = config.get_etherscan_config_with_chain(chain)? {
            trace!(target: "etherscanidentifier", chain=?config.chain, url=?config.api_url, "using etherscan identifier");
            Ok(Self {
                client: Some(Arc::new(config.into_client()?)),
                invalid_api_key: Arc::new(Default::default()),
            })
        } else {
            Ok(Default::default())
        }
    }
}

impl TraceIdentifier for EtherscanIdentifier {
    fn identify_addresses(
        &self,
        addresses: Vec<(&Address, Option<&Vec<u8>>)>,
    ) -> Vec<AddressIdentity> {
        trace!(target: "etherscanidentifier", "identify {} addresses", addresses.len());

        if self.invalid_api_key.load(Ordering::Relaxed) {
            // api key was marked as invalid
            return Vec::new()
        }

        self.client.as_ref().map_or(Default::default(), |client| {
            let mut fetcher = EtherscanFetcher::new(
                Arc::clone(client),
                Duration::from_secs(1),
                5,
                Arc::clone(&self.invalid_api_key),
            );

            for (addr, _) in addresses {
                fetcher.push(*addr);
            }

            let fut = fetcher
                .map(|(address, label, abi)| AddressIdentity {
                    address,
                    label: Some(label.clone()),
                    contract: Some(label),
                    abi: Some(Cow::Owned(abi)),
                    artifact_id: None,
                })
                .collect();

            RuntimeOrHandle::new().block_on(fut)
        })
    }
}

type EtherscanFuture =
    Pin<Box<dyn Future<Output = (Address, Result<ContractMetadata, EtherscanError>)>>>;

/// A rate limit aware Etherscan client.
///
/// Fetches information about multiple addresses concurrently, while respecting rate limits.
pub struct EtherscanFetcher {
    /// The Etherscan client
    client: Arc<etherscan::Client>,
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
        client: Arc<etherscan::Client>,
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
    type Item = (Address, String, Abi);

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
                                if let Ok(abi) = serde_json::from_str(&item.abi) {
                                    return Poll::Ready(Some((addr, item.contract_name, abi)))
                                }
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
                            pin.invalid_api_key.store(false, Ordering::Relaxed);
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
