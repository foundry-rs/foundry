use ethers::abi::{Event, Function};
use foundry_common::{
    abi::{get_event, get_func},
    fs,
    selectors::{SelectorType, SignEthClient},
};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;

pub type SingleSignaturesIdentifier = Arc<RwLock<SignaturesIdentifier>>;

/// An identifier that tries to identify functions and events using signatures found at
/// `sig.eth.samczsun.com`.
#[derive(Debug)]
pub struct SignaturesIdentifier {
    /// Cached selectors for functions and events
    cached: CachedSignatures,
    /// Location where to save `CachedSignatures`
    cached_path: Option<PathBuf>,
    /// Selectors that were unavailable during the session.
    unavailable: HashSet<Vec<u8>>,
    /// The API client to fetch signatures from
    sign_eth_api: SignEthClient,
    /// whether traces should be decoded via `sign_eth_api`
    offline: bool,
}

impl SignaturesIdentifier {
    #[instrument(target = "forge::signatures")]
    pub fn new(
        cache_path: Option<PathBuf>,
        offline: bool,
    ) -> eyre::Result<SingleSignaturesIdentifier> {
        let sign_eth_api = SignEthClient::new()?;

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
            Self {
                cached,
                cached_path: Some(path),
                unavailable: HashSet::new(),
                sign_eth_api,
                offline,
            }
        } else {
            Self {
                cached: Default::default(),
                cached_path: None,
                unavailable: HashSet::new(),
                sign_eth_api,
                offline,
            }
        };

        Ok(Arc::new(RwLock::new(identifier)))
    }

    #[instrument(target = "forge::signatures", skip(self))]
    pub fn save(&self) {
        if let Some(cached_path) = &self.cached_path {
            if let Some(parent) = cached_path.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    warn!(?parent, ?err, "failed to create cache");
                }
            }
            if let Err(err) = fs::write_json_file(cached_path, &self.cached) {
                warn!(?cached_path, ?err, "failed to flush signature cache");
            } else {
                trace!(?cached_path, "flushed signature cache")
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
        if self.unavailable.contains(identifier) {
            return None
        }

        let map = match selector_type {
            SelectorType::Function => &mut self.cached.functions,
            SelectorType::Event => &mut self.cached.events,
        };

        let hex_identifier = format!("0x{}", hex::encode(identifier));

        if !self.offline && !map.contains_key(&hex_identifier) {
            if let Ok(signatures) =
                self.sign_eth_api.decode_selector(&hex_identifier, selector_type).await
            {
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

    /// Returns `None` if in offline mode
    fn ensure_not_offline(&self) -> Option<()> {
        if self.offline {
            None
        } else {
            Some(())
        }
    }

    /// Identifies `Function` from its cache or `sig.eth.samczsun.com`
    pub async fn identify_function(&mut self, identifier: &[u8]) -> Option<Function> {
        self.ensure_not_offline()?;
        self.identify(SelectorType::Function, identifier, get_func).await
    }

    /// Identifies `Event` from its cache or `sig.eth.samczsun.com`
    pub async fn identify_event(&mut self, identifier: &[u8]) -> Option<Event> {
        self.ensure_not_offline()?;
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
