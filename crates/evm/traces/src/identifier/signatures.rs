use alloy_json_abi::{Error, Event, Function, JsonAbi};
use alloy_primitives::{
    B256, Selector,
    map::{HashMap, HashSet},
};
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
                // Only persist resolved signatures. Unknown selectors (None) are kept
                // in-memory for session dedup but not written to disk, so they can be
                // re-queried in future sessions once the signature database is updated.
                if let Some(value) = signature.clone() {
                    match *kind {
                        SelectorKind::Function(selector) => _ = acc.0.insert(selector, value),
                        SelectorKind::Error(selector) => _ = acc.1.insert(selector, value),
                        SelectorKind::Event(selector) => _ = acc.2.insert(selector, value),
                    }
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
        self.extend(Self::signatures_from_abi(abi));
    }

    /// Updates the cache from ABIs without overwriting previous entries from the same batch.
    fn extend_from_abis_without_collisions<'a>(
        &mut self,
        abis: impl IntoIterator<Item = &'a JsonAbi>,
    ) {
        let mut seeded: HashSet<SelectorKind> = HashSet::default();
        for abi in abis {
            for (selector, signature) in Self::signatures_from_abi(abi) {
                if seeded.insert(selector) {
                    self.insert(selector, signature);
                } else {
                    trace!(target: "evm::traces", ?selector, %signature, "skipping duplicate ABI signature");
                }
            }
        }
    }

    fn signatures_from_abi(abi: &JsonAbi) -> impl Iterator<Item = (SelectorKind, String)> + '_ {
        abi.items().filter_map(|item| match item {
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
        })
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
    /// ABI events keyed by topic0 and indexed topic count.
    local_events: HashMap<(B256, usize), Vec<Event>>,
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

    /// Creates an offline `SignaturesIdentifier` with the default cache directory and local ABIs.
    pub fn new_offline_with_abis<'a>(abis: impl IntoIterator<Item = &'a JsonAbi>) -> Result<Self> {
        Ok(Self::new_offline_with_abis_from_cache(Config::foundry_cache_dir().as_deref(), abis))
    }

    /// Creates a new `SignaturesIdentifier`.
    ///
    /// - `cache_dir` is the cache directory to store the signatures.
    /// - `offline` disables the OpenChain client.
    pub fn new_with(cache_dir: Option<&Path>, offline: bool) -> Result<Self> {
        let client = if offline { None } else { Some(OpenChainClient::new()?) };
        Ok(Self::from_cache(Self::load_cache(cache_dir), client))
    }

    fn new_offline_with_abis_from_cache<'a>(
        cache_dir: Option<&Path>,
        abis: impl IntoIterator<Item = &'a JsonAbi>,
    ) -> Self {
        let abis = abis.into_iter().collect::<Vec<_>>();
        let (mut cache, cache_path) = Self::load_cache(cache_dir);
        cache.extend_from_abis_without_collisions(abis.iter().copied());
        let local_events = Self::local_events_from_abis(abis);
        Self::from_cache_and_events((cache, cache_path), None, local_events)
    }

    fn load_cache(cache_dir: Option<&Path>) -> (SignaturesCache, Option<PathBuf>) {
        if let Some(cache_dir) = cache_dir {
            let path = cache_dir.join("signatures");
            let cache = SignaturesCache::load(&path);
            (cache, Some(path))
        } else {
            Default::default()
        }
    }

    fn from_cache(
        (cache, cache_path): (SignaturesCache, Option<PathBuf>),
        client: Option<OpenChainClient>,
    ) -> Self {
        Self::from_cache_and_events((cache, cache_path), client, Default::default())
    }

    fn from_cache_and_events(
        (cache, cache_path): (SignaturesCache, Option<PathBuf>),
        client: Option<OpenChainClient>,
        local_events: HashMap<(B256, usize), Vec<Event>>,
    ) -> Self {
        Self(Arc::new(SignaturesIdentifierInner {
            cache: RwLock::new(cache),
            local_events,
            cache_path,
            client,
        }))
    }

    fn local_events_from_abis<'a>(
        abis: impl IntoIterator<Item = &'a JsonAbi>,
    ) -> HashMap<(B256, usize), Vec<Event>> {
        let mut local_events: HashMap<(B256, usize), Vec<Event>> = HashMap::default();
        for abi in abis {
            for event in abi.events() {
                local_events
                    .entry((
                        event.selector(),
                        event.inputs.iter().filter(|input| input.indexed).count(),
                    ))
                    .or_default()
                    .push(event.clone());
            }
        }
        local_events
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

    /// Identifies an `Event`, preferring local ABI events with the matching indexed topic count.
    pub async fn identify_event_with_indexed_count(
        &self,
        identifier: B256,
        indexed_count: usize,
    ) -> Option<Event> {
        if let Some(events) = self.0.local_events.get(&(identifier, indexed_count))
            && let Some(event) = events.first()
        {
            return Some(event.clone());
        }
        self.identify_event(identifier).await
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
        // We only identify new signatures if the client is enabled.
        if let Some(path) = &self.cache_path
            && self.client.is_some()
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_signatures_not_persisted_to_disk() {
        let known_selector = SelectorKind::Function(Selector::from([0xaa, 0xbb, 0xcc, 0xdd]));
        let unknown_selector = SelectorKind::Error(Selector::from([0x11, 0x22, 0x33, 0x44]));

        let mut cache = SignaturesCache::default();
        cache.signatures.insert(known_selector, Some("transfer(address,uint256)".into()));
        cache.signatures.insert(unknown_selector, None);

        // Verify both are in memory.
        assert!(cache.contains_key(&known_selector));
        assert!(cache.contains_key(&unknown_selector));

        // Round-trip through the disk format.
        let disk: SignaturesDiskCache = (&cache).into();
        let reloaded = SignaturesCache::from(disk);

        // Known signature survives the round-trip.
        assert_eq!(reloaded.get(&known_selector), Some(Some("transfer(address,uint256)".into())));
        // Unknown signature is gone — it will be re-queried next session.
        assert_eq!(reloaded.get(&unknown_selector), None);
        assert!(!reloaded.contains_key(&unknown_selector));
    }

    #[tokio::test]
    async fn abi_seeded_signatures_are_not_persisted_to_disk() {
        let temp = tempfile::tempdir().unwrap();
        let event = Event::parse("event CodexEphemeral(uint256 indexed value)").unwrap();
        let mut abi = JsonAbi::default();
        abi.events.insert(event.name.clone(), vec![event.clone()]);

        {
            let identifier =
                SignaturesIdentifier::new_offline_with_abis_from_cache(Some(temp.path()), [&abi]);
            let decoded = identifier.identify_event(event.selector()).await;
            assert_eq!(decoded.as_ref().map(Event::full_signature), Some(event.full_signature()));
            identifier.save();
        }

        let reloaded = SignaturesCache::load(&temp.path().join("signatures"));
        assert!(!reloaded.contains_key(&SelectorKind::Event(event.selector())));
    }

    #[test]
    fn abi_seeded_collisions_keep_first_signature() {
        let first = Event::parse("event CodexCollision(uint256 indexed value)").unwrap();
        let second = Event::parse("event CodexCollision(uint256 value)").unwrap();

        let mut first_abi = JsonAbi::default();
        first_abi.events.insert(first.name.clone(), vec![first.clone()]);
        let mut second_abi = JsonAbi::default();
        second_abi.events.insert(second.name.clone(), vec![second]);

        let mut cache = SignaturesCache::default();
        cache.extend_from_abis_without_collisions([&first_abi, &second_abi]);

        assert_eq!(
            cache.get(&SelectorKind::Event(first.selector())),
            Some(Some(first.full_signature()))
        );
    }

    #[tokio::test]
    async fn abi_seeded_events_prefer_matching_indexed_count() {
        let one_topic =
            Event::parse("event CodexIndexedCount(uint256 indexed marker, uint256 value)").unwrap();
        let two_topics =
            Event::parse("event CodexIndexedCount(uint256 indexed marker, uint256 indexed value)")
                .unwrap();

        let mut one_topic_abi = JsonAbi::default();
        one_topic_abi.events.insert(one_topic.name.clone(), vec![one_topic.clone()]);
        let mut two_topics_abi = JsonAbi::default();
        two_topics_abi.events.insert(two_topics.name.clone(), vec![two_topics.clone()]);

        let identifier = SignaturesIdentifier::new_offline_with_abis_from_cache(
            None,
            [&two_topics_abi, &one_topic_abi],
        );

        let decoded_one_topic =
            identifier.identify_event_with_indexed_count(one_topic.selector(), 1).await.unwrap();
        let decoded_two_topics =
            identifier.identify_event_with_indexed_count(two_topics.selector(), 2).await.unwrap();

        assert_eq!(decoded_one_topic.full_signature(), one_topic.full_signature());
        assert_eq!(decoded_two_topics.full_signature(), two_topics.full_signature());
    }
}
