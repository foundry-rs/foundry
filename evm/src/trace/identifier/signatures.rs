use ethers::abi::{Event, Function};
use foundry_common::{
    abi::{get_event, get_func},
    fs,
    selectors::{decode_selector, SelectorType},
};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::BufWriter, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::{trace, warn};

pub type SingleSignaturesIdentifier = Arc<RwLock<SignaturesIdentifier>>;

/// An identifier that tries to identify functions and events using signatures found at
/// `sig.eth.samczsun.com`.
#[derive(Debug, Default)]
pub struct SignaturesIdentifier {
    // Cached selectors for functions and events
    cached: CachedSignatures,
    // Location where to save `CachedSignatures`
    cached_path: Option<PathBuf>,
    // Selectors that were unavailable during the session.
    unavailable: HashSet<Vec<u8>>,
}

impl SignaturesIdentifier {
    #[tracing::instrument(name = "signaturescache")]
    pub fn new(cache_path: Option<PathBuf>) -> eyre::Result<SingleSignaturesIdentifier> {
        let identifier = if let Some(cache_path) = cache_path {
            let path = cache_path.join("signatures");
            trace!(?path, "reading signature cache");
            let cached = if path.is_file() {
                fs::read_json_file(&path)
                    .map_err(|err| warn!(?path, ?err, "failed to read cache file"))
                    .unwrap_or_default()
            } else {
                if let Err(err) = std::fs::create_dir_all(cache_path) {
                    warn!("could not create signatures cache dir: {:?}", err);
                }
                CachedSignatures::default()
            };
            Self { cached, cached_path: Some(path), unavailable: HashSet::new() }
        } else {
            Self::default()
        };

        Ok(Arc::new(RwLock::new(identifier)))
    }

    pub fn save(&self) {
        if let Some(cached_path) = &self.cached_path {
            if let Ok(file) = std::fs::File::create(cached_path) {
                if serde_json::to_writer(BufWriter::new(file), &self.cached).is_err() {
                    warn!("could not serialize SignaturesIdentifier");
                }
            } else {
                warn!(?cached_path, "could not open cache file");
            }
        }
    }
}

impl SignaturesIdentifier {
    async fn identify<T>(
        &mut self,
        selector_type: SelectorType,
        identifier: &[u8],
        get_type: fn(&str) -> eyre::Result<T>,
    ) -> Option<T> {
        // Exit early if we have unsuccessfully queried it before.
        if self.unavailable.contains(&identifier.to_vec()) {
            return None
        }

        let map = match selector_type {
            SelectorType::Function => &mut self.cached.functions,
            SelectorType::Event => &mut self.cached.events,
        };

        let hex_identifier = format!("0x{}", hex::encode(identifier));

        if !map.contains_key(&hex_identifier) {
            if let Ok(signatures) = decode_selector(&hex_identifier, selector_type).await {
                if let Some(signature) = signatures.into_iter().next() {
                    map.insert(hex_identifier.clone(), signature);
                }
            }
        }

        if let Some(signature) = map.get(&hex_identifier) {
            return get_type(signature).ok()
        }

        self.unavailable.insert(identifier.to_vec());

        None
    }

    /// Identifies `Function` from its cache or `sig.eth.samczsun.com`
    pub async fn identify_function(&mut self, identifier: &[u8]) -> Option<Function> {
        self.identify(SelectorType::Function, identifier, get_func).await
    }

    /// Identifies `Event` from its cache or `sig.eth.samczsun.com`
    pub async fn identify_event(&mut self, identifier: &[u8]) -> Option<Event> {
        self.identify(SelectorType::Event, identifier, get_event).await
    }
}

impl Drop for SignaturesIdentifier {
    fn drop(&mut self) {
        self.save();
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct CachedSignatures {
    pub events: BTreeMap<String, String>,
    pub functions: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn can_query_signatures() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let sigs = SignaturesIdentifier::new(Some(tmp.path().into())).unwrap();

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

            assert!(func == get_func("transferFrom(address,address,uint256)").unwrap());
            assert!(event == get_event("Transfer(address,address,uint128)").unwrap());

            // dropping saves the cache
        }

        let sigs = SignaturesIdentifier::new(Some(tmp.path().into())).unwrap();
        assert!(sigs.read().await.cached.events.len() == 1);
        assert!(sigs.read().await.cached.functions.len() == 1);
    }
}
