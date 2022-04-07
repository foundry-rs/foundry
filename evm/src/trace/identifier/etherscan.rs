use super::{AddressIdentity, TraceIdentifier};
use ethers::{abi::Address, etherscan, types::Chain};
use futures::stream::{self, StreamExt};
use std::{borrow::Cow, path::PathBuf};
use tokio::time::{sleep, Duration};
use tracing::{trace, warn};

/// A trace identifier that tries to identify addresses using Etherscan.
pub struct EtherscanIdentifier {
    /// The Etherscan client
    client: Option<etherscan::Client>,
}

impl EtherscanIdentifier {
    pub fn new(
        chain: Option<impl Into<Chain>>,
        etherscan_api_key: String,
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
                etherscan::Client::new_cached(chain.into(), etherscan_api_key, cache_path, ttl).ok()
            }),
        }
    }
}

impl TraceIdentifier for EtherscanIdentifier {
    fn identify_addresses(
        &self,
        addresses: Vec<(&Address, Option<&Vec<u8>>)>,
    ) -> Vec<AddressIdentity> {
        if let Some(client) = &self.client {
            let stream = stream::iter(addresses.into_iter().map(futures::future::ready))
                .buffered(10)
                .filter_map(|(addr, _)| {
                    let client = client.clone();
                    async move {
                        let mut i = 0;
                        trace!(target: "etherscanidentifier", "requesting etherscan info for contract {:?}", addr);
                        loop {
                            match client.contract_source_code(*addr).await {
                                Ok(mut metadata) => {
                                    break metadata.items.pop().and_then(|item| {
                                        Some((
                                            *addr,
                                            item.contract_name,
                                            serde_json::from_str(&item.abi).ok()?,
                                        ))
                                    })
                                }
                                Err(etherscan::errors::EtherscanError::RateLimitExceeded) => {
                                    sleep(Duration::from_secs(1)).await;
                                    trace!(target: "etherscanidentifier", "rate limit exceeded on attempt {}", i);
                                    i += 1;
                                    if i < 5 {
                                        continue
                                    } else {
                                        warn!(target: "etherscanidentifier", "no more retries left for request");
                                        break None
                                    }
                                }
                                Err(err) => {
                                    warn!(target: "etherscanidentifier", "could not get etherscan info: {:?}", err);
                                    break None
                                }
                            }
                        }
                    }
                })
                .map(|(address, label, abi): (Address, String, ethers::abi::Abi)| {
                    AddressIdentity { address, label: Some(label.clone()), contract: Some(label), abi: Some(Cow::Owned(abi)) }
                })
                .collect();
            foundry_utils::RuntimeOrHandle::new().block_on(stream)
        } else {
            Vec::new()
        }
    }
}
