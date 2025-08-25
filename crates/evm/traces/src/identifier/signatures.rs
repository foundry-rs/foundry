use alloy_json_abi::{Error, Event, Function, JsonAbi};
use alloy_primitives::{B256, Selector, map::HashMap};
use eyre::Result;
use foundry_common::{
    abi::{get_error, get_event, get_func},
    fs,
    selectors::{OpenChainClient, SelectorKind},
};
use foundry_config::Config;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;

/// Cache for function, event and error signatures. Used by [`SignaturesIdentifier`].
#[derive(Debug, Default, Deserialize)]
#[serde(try_from = "SignaturesDiskCache")]
pub struct SignaturesCache {
    signatures: HashMap<SelectorKind, Option<String>>,
}

/// Disk representation of the signatures cache.
#[derive(Serialize, Deserialize)]
struct SignaturesDiskCache {
    functions: BTreeMap<Selector, String>,
    errors: BTreeMap<Selector, String>,
    events: BTreeMap<B256, String>,
}

impl From<SignaturesDiskCache> for SignaturesCache {
    fn from(value: SignaturesDiskCache) -> Self {
        let functions = value
            .functions
            .into_iter()
            .map(|(selector, signature)| (SelectorKind::Function(selector), signature));
        let errors = value
            .errors
            .into_iter()
            .map(|(selector, signature)| (SelectorKind::Error(selector), signature));
        let events = value
            .events
            .into_iter()
            .map(|(selector, signature)| (SelectorKind::Event(selector), signature));
        Self {
            signatures: functions
                .chain(errors)
                .chain(events)
                .map(|(sel, sig)| (sel, (!sig.is_empty()).then_some(sig)))
                .collect(),
        }
    }
}

impl From<&SignaturesCache> for SignaturesDiskCache {
    fn from(value: &SignaturesCache) -> Self {
        let (functions, errors, events) = value.signatures.iter().fold(
            (BTreeMap::new(), BTreeMap::new(), BTreeMap::new()),
            |mut acc, (kind, signature)| {
                let value = signature.clone().unwrap_or_default();
                match *kind {
                    SelectorKind::Function(selector) => _ = acc.0.insert(selector, value),
                    SelectorKind::Error(selector) => _ = acc.1.insert(selector, value),
                    SelectorKind::Event(selector) => _ = acc.2.insert(selector, value),
                }
                acc
            },
        );
        Self { functions, errors, events }
    }
}

impl Serialize for SignaturesCache {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SignaturesDiskCache::from(self).serialize(serializer)
    }
}

impl SignaturesCache {
    /// Loads the cache from a file.
    #[instrument(target = "evm::traces", name = "SignaturesCache::load")]
    pub fn load(path: &Path) -> Self {
        trace!(target: "evm::traces", ?path, "reading signature cache");
        fs::read_json_file(path)
            .inspect_err(
                |err| warn!(target: "evm::traces", ?path, ?err, "failed to read cache file"),
            )
            .unwrap_or_default()
    }

    /// Saves the cache to a file.
    #[instrument(target = "evm::traces", name = "SignaturesCache::save", skip(self))]
    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            warn!(target: "evm::traces", ?parent, %err, "failed to create cache");
        }
        if let Err(err) = fs::write_json_file(path, self) {
            warn!(target: "evm::traces", %err, "failed to flush signature cache");
        } else {
            trace!(target: "evm::traces", "flushed signature cache")
        }
    }

    /// Updates the cache from an ABI.
    pub fn extend_from_abi(&mut self, abi: &JsonAbi) {
        self.extend(abi.items().filter_map(|item| match item {
            alloy_json_abi::AbiItem::Function(f) => {
                Some((SelectorKind::Function(f.selector()), f.signature()))
            }
            alloy_json_abi::AbiItem::Error(e) => {
                Some((SelectorKind::Error(e.selector()), e.signature()))
            }
            alloy_json_abi::AbiItem::Event(e) => {
                Some((SelectorKind::Event(e.selector()), e.full_signature()))
            }
            _ => None,
        }));
    }

    /// Inserts a single signature into the cache.
    pub fn insert(&mut self, key: SelectorKind, value: String) {
        self.extend(std::iter::once((key, value)));
    }

    /// Extends the cache with multiple signatures.
    pub fn extend(&mut self, signatures: impl IntoIterator<Item = (SelectorKind, String)>) {
        self.signatures
            .extend(signatures.into_iter().map(|(k, v)| (k, (!v.is_empty()).then_some(v))));
    }

    /// Gets a signature from the cache.
    pub fn get(&self, key: &SelectorKind) -> Option<Option<String>> {
        self.signatures.get(key).cloned()
    }

    /// Returns true if the cache contains a signature.
    pub fn contains_key(&self, key: &SelectorKind) -> bool {
        self.signatures.contains_key(key)
    }
}

/// An identifier that tries to identify functions and events using signatures found at
/// `https://openchain.xyz` or a local cache.
#[derive(Clone, Debug)]
pub struct SignaturesIdentifier(Arc<SignaturesIdentifierInner>);

#[derive(Debug)]
struct SignaturesIdentifierInner {
    /// Cached selectors for functions, events and custom errors.
    cache: RwLock<SignaturesCache>,
    /// Location where to save the signature cache.
    cache_path: Option<PathBuf>,
    /// The OpenChain client to fetch signatures from. `None` if disabled on construction.
    client: Option<OpenChainClient>,
}

impl SignaturesIdentifier {
    /// Creates a new `SignaturesIdentifier` with the default cache directory.
    pub fn new(offline: bool) -> Result<Self> {
        Self::new_with(Config::foundry_cache_dir().as_deref(), offline)
    }

    /// Creates a new `SignaturesIdentifier` from the global configuration.
    pub fn from_config(config: &Config) -> Result<Self> {
        Self::new(config.offline)
    }

    /// Creates a new `SignaturesIdentifier`.
    ///
    /// - `cache_dir` is the cache directory to store the signatures.
    /// - `offline` disables the OpenChain client.
    pub fn new_with(cache_dir: Option<&Path>, offline: bool) -> Result<Self> {
        let client = if !offline { Some(OpenChainClient::new()?) } else { None };
        let (cache, cache_path) = if let Some(cache_dir) = cache_dir {
            let path = cache_dir.join("signatures");
            let cache = SignaturesCache::load(&path);
            (cache, Some(path))
        } else {
            Default::default()
        };
        Ok(Self(Arc::new(SignaturesIdentifierInner {
            cache: RwLock::new(cache),
            cache_path,
            client,
        })))
    }

    /// Saves the cache to the file system.
    pub fn save(&self) {
        self.0.save();
    }

    /// Identifies `Function`s.
    pub async fn identify_functions(
        &self,
        identifiers: impl IntoIterator<Item = Selector>,
    ) -> Vec<Option<Function>> {
        self.identify_map(identifiers.into_iter().map(SelectorKind::Function), get_func).await
    }

    /// Identifies a `Function`.
    pub async fn identify_function(&self, identifier: Selector) -> Option<Function> {
        self.identify_functions([identifier]).await.pop().unwrap()
    }

    /// Identifies `Event`s.
    pub async fn identify_events(
        &self,
        identifiers: impl IntoIterator<Item = B256>,
    ) -> Vec<Option<Event>> {
        self.identify_map(identifiers.into_iter().map(SelectorKind::Event), get_event).await
    }

    /// Identifies an `Event`.
    pub async fn identify_event(&self, identifier: B256) -> Option<Event> {
        self.identify_events([identifier]).await.pop().unwrap()
    }

    /// Identifies `Error`s.
    pub async fn identify_errors(
        &self,
        identifiers: impl IntoIterator<Item = Selector>,
    ) -> Vec<Option<Error>> {
        self.identify_map(identifiers.into_iter().map(SelectorKind::Error), get_error).await
    }

    /// Identifies an `Error`.
    pub async fn identify_error(&self, identifier: Selector) -> Option<Error> {
        self.identify_errors([identifier]).await.pop().unwrap()
    }

    /// Identifies a list of selectors.
    pub async fn identify(&self, selectors: &[SelectorKind]) -> Vec<Option<String>> {
        if selectors.is_empty() {
            return vec![];
        }
        trace!(target: "evm::traces", ?selectors, "identifying selectors");

        let mut cache_r = self.0.cache.read().await;
        if let Some(client) = &self.0.client {
            let query =
                selectors.iter().copied().filter(|v| !cache_r.contains_key(v)).collect::<Vec<_>>();
            if !query.is_empty() {
                drop(cache_r);
                let mut cache_w = self.0.cache.write().await;
                if let Ok(res) = client.decode_selectors(&query).await {
                    for (selector, signatures) in std::iter::zip(query, res) {
                        cache_w.signatures.insert(selector, signatures.into_iter().next());
                    }
                }
                drop(cache_w);
                cache_r = self.0.cache.read().await;
            }
        }
        selectors.iter().map(|selector| cache_r.get(selector).unwrap_or_default()).collect()
    }

    async fn identify_map<T>(
        &self,
        selectors: impl IntoIterator<Item = SelectorKind>,
        get_type: impl Fn(&str) -> Result<T>,
    ) -> Vec<Option<T>> {
        let results = self.identify(&Vec::from_iter(selectors)).await;
        results.into_iter().map(|r| r.and_then(|r| get_type(&r).ok())).collect()
    }
}

impl SignaturesIdentifierInner {
    fn save(&self) {
        if let Some(path) = &self.cache_path {
            self.cache
                .try_read()
                .expect("SignaturesIdentifier cache is locked while attempting to save")
                .save(path);
        }
    }
}

impl Drop for SignaturesIdentifierInner {
    fn drop(&mut self) {
        self.save();
    }
}
