use super::TraceIdentifier;
use ethers::{
    abi::{Abi, Address},
    etherscan,
    types::Chain,
};
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
    ) -> Self {
        if let Some(cache_path) = &cache_path {
            if let Err(err) = std::fs::create_dir_all(cache_path.join("sources")) {
                warn!("could not create etherscan cache dir: {:?}", err);
            }
        }

        Self {
            client: chain
                .map(|chain| {
                    etherscan::Client::new_cached(chain.into(), etherscan_api_key, cache_path).ok()
                })
                .flatten(),
        }
    }
}

impl TraceIdentifier for EtherscanIdentifier {
    fn identify_addresses(
        &self,
        addresses: Vec<(&Address, Option<&Vec<u8>>)>,
    ) -> Vec<(Address, Option<String>, Option<String>, Option<Cow<Abi>>)> {
        if let Some(client) = &self.client {
            let stream = stream::iter(addresses.into_iter().map(futures::future::ready))
                .buffered(10)
                .filter_map(|(addr, _)| {
                    let client = client.clone();
                    async move {
                        let mut i = 0;
                        trace!("requesting etherscan info for contract {addr}");
                        loop {
                            match client.contract_source_code(*addr).await {
                                Ok(mut metadata) => {
                                    break metadata
                                        .items
                                        .pop()
                                        .map(|item| {
                                            Some((
                                                *addr,
                                                item.contract_name,
                                                serde_json::from_str(&item.abi).ok()?,
                                            ))
                                        })
                                        .flatten()
                                }
                                Err(etherscan::errors::EtherscanError::RateLimitExceeded) => {
                                    sleep(Duration::from_secs(1)).await;
                                    trace!("rate limit exceeded on attempt {i}");
                                    i += 1;
                                    if i < 5 {
                                        continue
                                    } else {
                                        warn!("no more retries left for request");
                                        break None
                                    }
                                }
                                Err(err) => {
                                    warn!("could not get etherscan info: {:?}", err);
                                    break None
                                }
                            }
                        }
                    }
                })
                .map(|(addr, label, abi): (Address, String, ethers::abi::Abi)| {
                    (addr, Some(label.clone()), Some(label.clone()), Some(Cow::Owned(abi.clone())))
                })
                .collect();
            let rt = tokio::runtime::Runtime::new().expect("could not start tokio rt");
            rt.block_on(stream)
        } else {
            Vec::new()
        }
    }
}
