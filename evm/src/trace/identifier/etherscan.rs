use super::{AddressIdentity, TraceIdentifier};
use ethers::{
    abi::{Abi, Address},
    etherscan,
    prelude::{contract::ContractMetadata, errors::EtherscanError},
    solc::utils::RuntimeOrHandle,
    types::Chain,
};
use futures::{
    future::Future,
    stream::{FuturesUnordered, Stream, StreamExt},
    task::{Context, Poll},
};
use std::{borrow::Cow, path::PathBuf, pin::Pin};
use tokio::time::{Duration, Interval};
use tracing::{trace, warn};

/// A trace identifier that tries to identify addresses using Etherscan.
pub struct EtherscanIdentifier {
    /// The Etherscan client
    client: Option<etherscan::Client>,
}

impl EtherscanIdentifier {
    /// Creates a new Etherscan identifier.
    ///
    /// The identifier is a noop if either `chain` or `etherscan_api_key` are `None`.
    pub fn new(
        chain: Option<impl Into<Chain>>,
        etherscan_api_key: Option<String>,
        cache_path: Option<PathBuf>,
        ttl: Duration,
    ) -> Self {
        if let Some(cache_path) = &cache_path {
            if let Err(err) = std::fs::create_dir_all(cache_path.join("sources")) {
                warn!(target: "etherscanidentifier", "could not create etherscan cache dir: {:?}", err);
            }
        }

        Self {
            client: chain.and_then(|chain| {
                etherscan_api_key.and_then(|key| {
                    etherscan::Client::new_cached(chain.into(), key, cache_path, ttl).ok()
                })
            }),
        }
    }
}

impl TraceIdentifier for EtherscanIdentifier {
    fn identify_addresses(
        &self,
        addresses: Vec<(&Address, Option<&Vec<u8>>)>,
    ) -> Vec<AddressIdentity> {
        self.client.as_ref().map_or(Default::default(), |client| {
            let mut fetcher = EtherscanFetcher::new(client.clone(), Duration::from_secs(1), 5);

            for (addr, _) in addresses {
                fetcher.push(*addr);
            }

            let fut = fetcher
                .map(|(address, label, abi)| AddressIdentity {
                    address,
                    label: Some(label.clone()),
                    contract: Some(label),
                    abi: Some(Cow::Owned(abi)),
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
    client: etherscan::Client,
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
}

impl EtherscanFetcher {
    pub fn new(client: etherscan::Client, timeout: Duration, concurrency: usize) -> Self {
        Self {
            client,
            timeout,
            backoff: None,
            concurrency,
            queue: Vec::new(),
            in_progress: FuturesUnordered::new(),
        }
    }

    pub fn push(&mut self, address: Address) {
        self.queue.push(address);
    }

    fn queue_next_reqs(&mut self) {
        while self.in_progress.len() < self.concurrency {
            if let Some(addr) = self.queue.pop() {
                let client = self.client.clone();
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
                        Err(etherscan::errors::EtherscanError::RateLimitExceeded) => {
                            warn!(target: "etherscanidentifier", "rate limit exceeded on attempt");
                            pin.backoff = Some(tokio::time::interval(pin.timeout));
                            pin.queue.push(addr);
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
