use super::{IdentifiedAddress, TraceIdentifier};
use crate::debug::ContractSources;
use alloy_json_abi::JsonAbi;
use alloy_primitives::Address;
use foundry_common::compile::etherscan_project;
use foundry_config::{Chain, Config};
use futures::{
    future::join_all,
    stream::{FuturesUnordered, Stream, StreamExt},
    task::{Context, Poll},
};
use reqwest::StatusCode;
use revm_inspectors::tracing::types::CallTraceNode;
use serde::Deserialize;
use std::{borrow::Cow, collections::BTreeMap, pin::Pin, sync::atomic::Ordering};
use tokio::time::{Duration, Interval};

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum SourcifyResponse {
    Success(Metadata),
    Error(SourcifyErrorResponse),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcifyErrorResponse {
    custom_code: String,
    message: String,
    error_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    #[serde(default)]
    abi: Option<JsonAbi>,
    #[serde(default)]
    compilation: Option<Compilation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Compilation {
    #[serde(default)]
    language: String,
    #[serde(default)]
    name: String,
}

/// A trace identifier that tries to identify addresses using Etherscan.
pub struct SourcifyIdentifier {
    client: reqwest::Client,
    url: String,
    contracts: BTreeMap<Address, Metadata>,
}

impl SourcifyIdentifier {
    /// Creates a new Etherscan identifier with the given client
    pub fn new(config: &Config, chain: Option<Chain>) -> eyre::Result<Option<Self>> {
        if config.offline {
            return Ok(None);
        }
        Ok(Some(Self {
            client: reqwest::Client::new(),
            url: format!(
                "https://sourcify.dev/server/v2/contract/{}",
                chain.unwrap_or_default().id(),
            ),
            contracts: BTreeMap::new(),
        }))
    }

    fn identify_from_metadata(
        &self,
        address: Address,
        metadata: &Metadata,
    ) -> IdentifiedAddress<'static> {
        let label = metadata.compilation.as_ref().map(|c| c.name.clone());
        let abi = metadata.abi.clone().map(Cow::Owned);
        IdentifiedAddress { address, label: label.clone(), contract: label, abi, artifact_id: None }
    }
}

impl TraceIdentifier for SourcifyIdentifier {
    fn identify_addresses(&mut self, nodes: &[&CallTraceNode]) -> Vec<IdentifiedAddress<'_>> {
        if nodes.is_empty() {
            return Vec::new();
        }

        trace!(target: "evm::traces::etherscan", "identify {} addresses", nodes.len());

        let mut identities = Vec::new();
        let mut fetcher =
            SourcifyFetcher::new(self.client.clone(), self.url.clone(), Duration::from_secs(1), 5);

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

type SourcifyFuture = Pin<
    Box<dyn Future<Output = (Address, Result<(StatusCode, SourcifyResponse), reqwest::Error>)>>,
>;

/// A rate limit aware Sourcify client.
///
/// Fetches information about multiple addresses concurrently, while respecting rate limits.
struct SourcifyFetcher {
    /// The client
    client: reqwest::Client,
    /// The URL to fetch the contract metadata from
    url: String,
    /// The time we wait if we hit the rate limit
    timeout: Duration,
    /// The interval we are currently waiting for before making a new request
    backoff: Option<Interval>,
    /// The maximum amount of requests to send concurrently
    concurrency: usize,
    /// The addresses we have yet to make requests for
    queue: Vec<Address>,
    /// The in progress requests
    in_progress: FuturesUnordered<SourcifyFuture>,
}

impl SourcifyFetcher {
    fn new(client: reqwest::Client, url: String, timeout: Duration, concurrency: usize) -> Self {
        Self {
            client,
            url,
            timeout,
            backoff: None,
            concurrency,
            queue: Vec::new(),
            in_progress: FuturesUnordered::new(),
        }
    }

    fn push(&mut self, address: Address) {
        self.queue.push(address);
    }

    fn queue_next_reqs(&mut self) {
        while self.in_progress.len() < self.concurrency {
            let Some(addr) = self.queue.pop() else { break };
            let client = self.client.clone();
            let url = self.url.clone();
            self.in_progress.push(Box::pin(async move {
                trace!(target: "traces::etherscan", ?addr, "fetching info");
                let res = client.get(format!("{url}/{addr}?fields=abi")).send().await;
                let res = match res {
                    Ok(res) => {
                        let code = res.status();
                        res.json().await.map(|res| (code, res))
                    }
                    Err(e) => Err(e),
                };
                (addr, res)
            }));
        }
    }
}

impl Stream for SourcifyFetcher {
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
                    let (code, res) = match res {
                        Ok(r) => r,
                        Err(err) => {
                            warn!(target: "traces::etherscan", ?err, "could not get sourcify info");
                            // continue;
                        }
                    };
                    if code.as_u16() == 429 {
                        warn!(target: "traces::sourcify", ?res, "rate limit exceeded on attempt");
                        pin.backoff = Some(tokio::time::interval(pin.timeout));
                        pin.queue.push(addr);
                        // continue;
                    }
                    if let SourcifyResponse::Success(res) = res {
                        return Poll::Ready(Some((addr, res)));
                    }
                }
            }

            if !made_progress_this_iter {
                return Poll::Pending;
            }
        }
    }
}
