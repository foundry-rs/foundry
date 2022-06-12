use ethers::abi::{Event, Function};
use foundry_utils::{decode_selector, get_event, get_func, selectors::SelectorType};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::BufWriter, path::PathBuf};
use tracing::warn;

/// An identifier that tries to identify functions and events using signatures found at
/// `sig.eth.samczsun.com`.
#[derive(Debug, Default)]
pub struct SignaturesIdentifier {
    cached: CachedSignatures,
    cached_path: Option<PathBuf>,
}

impl SignaturesIdentifier {
    pub fn new(cache_path: Option<PathBuf>) -> eyre::Result<Self> {
        if let Some(cache_path) = cache_path {
            let path = cache_path.join("signatures");
            let cached = if path.is_file() {
                serde_json::from_reader(std::fs::File::open(&path)?)?
            } else {
                if let Err(err) = std::fs::create_dir_all(cache_path) {
                    warn!(target: "signaturesidentifier", "could not create signatures cache dir: {:?}", err);
                }
                CachedSignatures::default()
            };
            return Ok(Self { cached, cached_path: Some(path) })
        }
        Ok(Self::default())
    }

    pub fn save(&self) {
        if let Some(cached_path) = &self.cached_path {
            if let Ok(file) = std::fs::File::create(&cached_path) {
                if serde_json::to_writer(BufWriter::new(file), &self.cached).is_err() {
                    warn!(target: "signaturesidentifier", "could not serialize SignaturesIdentifier");
                }
            } else {
                warn!(target: "signaturesidentifier", "could not open {}", cached_path.to_string_lossy());
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
        let map = match selector_type {
            SelectorType::Function => &mut self.cached.functions,
            SelectorType::Event => &mut self.cached.events,
        };

        let identifier = format!("0x{}", hex::encode(identifier));

        if !map.contains_key(&identifier) {
            if let Ok(signatures) = decode_selector(&identifier, selector_type).await {
                if let Some(signature) = signatures.into_iter().next() {
                    map.insert(identifier.to_string(), signature);
                }
            }
        }

        if let Some(signature) = map.get(&identifier) {
            return get_type(signature).ok()
        }
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
            let mut sigs = SignaturesIdentifier::new(Some(tmp.path().into())).unwrap();

            assert!(sigs.cached.events.is_empty());
            assert!(sigs.cached.functions.is_empty());

            let func = sigs.identify_function(&[35, 184, 114, 221]).await.unwrap();
            let event = sigs
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
        assert!(sigs.cached.events.len() == 1);
        assert!(sigs.cached.functions.len() == 1);
    }
}
