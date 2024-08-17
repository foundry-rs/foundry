use alloy_json_abi::{Event, Function};
use alloy_primitives::hex;
use foundry_common::{
    abi::{get_event, get_func},
    fs,
    selectors::{OpenChainClient, SelectorType},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::RwLock;

pub type SingleSignaturesIdentifier = Arc<RwLock<SignaturesIdentifier>>;

#[derive(Debug, Default, Serialize, Deserialize)]
struct CachedSignatures {
    events: BTreeMap<String, String>,
    functions: BTreeMap<String, String>,
}

/// An identifier that tries to identify functions and events using signatures found at
/// `https://openchain.xyz` or a local cache.
#[derive(Debug)]
pub struct SignaturesIdentifier {
    /// Cached selectors for functions and events.
    cached: CachedSignatures,
    /// Location where to save `CachedSignatures`.
    cached_path: Option<PathBuf>,
    /// Selectors that were unavailable during the session.
    unavailable: HashSet<String>,
    /// The OpenChain client to fetch signatures from.
    client: Option<OpenChainClient>,
}

impl SignaturesIdentifier {
    #[instrument(target = "evm::traces")]
    pub fn new(
        cache_path: Option<PathBuf>,
        offline: bool,
    ) -> eyre::Result<SingleSignaturesIdentifier> {
        let client = if !offline { Some(OpenChainClient::new()?) } else { None };

        let identifier = if let Some(cache_path) = cache_path {
            let path = cache_path.join("signatures");
            trace!(target: "evm::traces", ?path, "reading signature cache");
            let cached = if path.is_file() {
                fs::read_json_file(&path)
                    .map_err(|err| warn!(target: "evm::traces", ?path, ?err, "failed to read cache file"))
                    .unwrap_or_default()
            } else {
                if let Err(err) = std::fs::create_dir_all(cache_path) {
                    warn!(target: "evm::traces", "could not create signatures cache dir: {:?}", err);
                }
                CachedSignatures::default()
            };
            Self { cached, cached_path: Some(path), unavailable: HashSet::new(), client }
        } else {
            Self {
                cached: Default::default(),
                cached_path: None,
                unavailable: HashSet::new(),
                client,
            }
        };

        Ok(Arc::new(RwLock::new(identifier)))
    }

    #[instrument(target = "evm::traces", skip(self))]
    pub fn save(&self) {
        if let Some(cached_path) = &self.cached_path {
            if let Some(parent) = cached_path.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    warn!(target: "evm::traces", ?parent, ?err, "failed to create cache");
                }
            }
            if let Err(err) = fs::write_json_file(cached_path, &self.cached) {
                warn!(target: "evm::traces", ?cached_path, ?err, "failed to flush signature cache");
            } else {
                trace!(target: "evm::traces", ?cached_path, "flushed signature cache")
            }
        }
    }
}

impl SignaturesIdentifier {
    async fn identify<T>(
        &mut self,
        selector_type: SelectorType,
        identifiers: impl IntoIterator<Item = impl AsRef<[u8]>>,
        get_type: impl Fn(&str) -> eyre::Result<T>,
    ) -> Vec<Option<T>> {
        let cache = match selector_type {
            SelectorType::Function => &mut self.cached.functions,
            SelectorType::Event => &mut self.cached.events,
        };

        let hex_identifiers: Vec<String> =
            identifiers.into_iter().map(hex::encode_prefixed).collect();

        if let Some(client) = &self.client {
            let query: Vec<_> = hex_identifiers
                .iter()
                .filter(|v| !cache.contains_key(v.as_str()))
                .filter(|v| !self.unavailable.contains(v.as_str()))
                .collect();

            if let Ok(res) = client.decode_selectors(selector_type, query.clone()).await {
                for (hex_id, selector_result) in query.into_iter().zip(res.into_iter()) {
                    let mut found = false;
                    if let Some(decoded_results) = selector_result {
                        if let Some(decoded_result) = decoded_results.into_iter().next() {
                            cache.insert(hex_id.clone(), decoded_result);
                            found = true;
                        }
                    }
                    if !found {
                        self.unavailable.insert(hex_id.clone());
                    }
                }
            }
        }

        hex_identifiers.iter().map(|v| cache.get(v).and_then(|v| get_type(v).ok())).collect()
    }

    /// Identifies `Function`s from its cache or `https://api.openchain.xyz`
    pub async fn identify_functions(
        &mut self,
        identifiers: impl IntoIterator<Item = impl AsRef<[u8]>>,
    ) -> Vec<Option<Function>> {
        self.identify(SelectorType::Function, identifiers, get_func).await
    }

    /// Identifies `Function` from its cache or `https://api.openchain.xyz`
    pub async fn identify_function(&mut self, identifier: &[u8]) -> Option<Function> {
        self.identify_functions(&[identifier]).await.pop().unwrap()
    }

    /// Identifies `Event`s from its cache or `https://api.openchain.xyz`
    pub async fn identify_events(
        &mut self,
        identifiers: impl IntoIterator<Item = impl AsRef<[u8]>>,
    ) -> Vec<Option<Event>> {
        self.identify(SelectorType::Event, identifiers, get_event).await
    }

    /// Identifies `Event` from its cache or `https://api.openchain.xyz`
    pub async fn identify_event(&mut self, identifier: &[u8]) -> Option<Event> {
        self.identify_events(&[identifier]).await.pop().unwrap()
    }
}

impl Drop for SignaturesIdentifier {
    fn drop(&mut self) {
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn can_query_signatures() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let sigs = SignaturesIdentifier::new(Some(tmp.path().into()), false).unwrap();

            assert!(sigs.read().await.cached.events.is_empty());
            assert!(sigs.read().await.cached.functions.is_empty());

            let func = sigs.write().await.identify_function(&[35, 184, 114, 221]).await.unwrap();
            let event = sigs
                .write()
                .await
                .identify_event(&[
                    39, 119, 42, 220, 99, 219, 7, 170, 231, 101, 183, 30, 178, 181, 51, 6, 79, 167,
                    129, 189, 87, 69, 126, 27, 19, 133, 146, 216, 25, 141, 9, 89,
                ])
                .await
                .unwrap();

            assert_eq!(func, get_func("transferFrom(address,address,uint256)").unwrap());
            assert_eq!(event, get_event("Transfer(address,address,uint128)").unwrap());

            // dropping saves the cache
        }

        let sigs = SignaturesIdentifier::new(Some(tmp.path().into()), false).unwrap();
        assert_eq!(sigs.read().await.cached.events.len(), 1);
        assert_eq!(sigs.read().await.cached.functions.len(), 1);
    }
}
