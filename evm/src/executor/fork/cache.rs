//! Cache related abstraction
use ethers::types::{Address, H256, U256};
use parking_lot::RwLock;
use revm::AccountInfo;
use serde::{ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, fs, io::BufWriter, path::PathBuf, sync::Arc};
use tracing::{trace, trace_span, warn};
use tracing_error::InstrumentResult;

pub type StorageInfo = BTreeMap<U256, U256>;

/// A shareable Block database
#[derive(Clone, Debug)]
pub struct BlockchainDb {
    /// Contains all the data
    db: Arc<MemDb>,
    /// metadata of the current config
    meta: Arc<RwLock<BlockchainDbMeta>>,
    /// the cache that can be flushed
    cache: Arc<JsonBlockCacheDB>,
}

impl BlockchainDb {
    /// Creates a new instance of the [BlockchainDb]
    ///
    /// if a `cache_path` is provided it attempts to load a previously stored [JsonBlockCacheData]
    /// and will try to use the cached entries it holds.
    ///
    /// This will return a new and empty [MemDb] if
    ///   - `cache_path` is `None`
    ///   - the file the `cache_path` points to, does not exist
    ///   - the file contains malformed data, or if it couldn't be read
    ///   - the provided `meta` differs from [BlockchainDbMeta] that's stored on disk
    pub fn new(meta: BlockchainDbMeta, cache_path: Option<PathBuf>) -> Self {
        // read cache and check if metadata matches
        let cache = cache_path
            .as_ref()
            .and_then(|p| {
                JsonBlockCacheDB::load(p).ok().filter(|cache| {
                    if meta != *cache.meta().read() {
                        warn!(target:"cache", "non-matching block metadata");
                        false
                    } else {
                        true
                    }
                })
            })
            .unwrap_or_else(|| JsonBlockCacheDB::new(Arc::new(RwLock::new(meta)), cache_path));

        Self { db: Arc::clone(cache.db()), meta: Arc::clone(cache.meta()), cache: Arc::new(cache) }
    }

    /// Returns the map that holds the account related info
    pub fn accounts(&self) -> &RwLock<BTreeMap<Address, AccountInfo>> {
        &self.db.accounts
    }

    /// Returns the map that holds the storage related info
    pub fn storage(&self) -> &RwLock<BTreeMap<Address, StorageInfo>> {
        &self.db.storage
    }

    /// Returns the map that holds all the block hashes
    pub fn block_hashes(&self) -> &RwLock<BTreeMap<u64, H256>> {
        &self.db.block_hashes
    }

    /// Returns the [revm::Env] related metadata
    pub fn meta(&self) -> &Arc<RwLock<BlockchainDbMeta>> {
        &self.meta
    }

    /// Returns the inner cache
    pub fn cache(&self) -> &Arc<JsonBlockCacheDB> {
        &self.cache
    }
}

/// relevant identifying markers in the context of [BlockchainDb]
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockchainDbMeta {
    pub cfg_env: revm::CfgEnv,
    pub block_env: revm::BlockEnv,
    pub host: String,
}

/// In Memory cache containing all fetched accounts and storage slots
/// and their values from RPC
#[derive(Debug, Default)]
pub struct MemDb {
    /// Account related data
    pub accounts: RwLock<BTreeMap<Address, AccountInfo>>,
    /// Storage related data
    pub storage: RwLock<BTreeMap<Address, StorageInfo>>,
    /// All retrieved block hashes
    pub block_hashes: RwLock<BTreeMap<u64, H256>>,
}

/// A [BlockCacheDB] that stores the cached content in a json file
#[derive(Debug)]
pub struct JsonBlockCacheDB {
    /// Where this cache file is stored.
    ///
    /// If this is a [None] then caching is disabled
    cache_path: Option<PathBuf>,
    /// Object that's stored in a json file
    data: JsonBlockCacheData,
}

impl JsonBlockCacheDB {
    /// Creates a new instance.
    fn new(meta: Arc<RwLock<BlockchainDbMeta>>, cache_path: Option<PathBuf>) -> Self {
        Self { cache_path, data: JsonBlockCacheData { meta, data: Arc::new(Default::default()) } }
    }

    /// Loads the contents of the diskmap file and returns the read object
    ///
    /// # Errors
    /// This will fail if
    ///   - the `path` does not exist
    ///   - the format does not match [JsonBlockCacheData]
    pub fn load(path: impl Into<PathBuf>) -> eyre::Result<Self> {
        let path = path.into();
        trace!(target: "cache", "reading json cache path={:?}", path);
        let span = trace_span!("cache", "path={:?}", &path);
        let _enter = span.enter();
        let file = std::fs::File::open(&path).in_current_span()?;
        let file = std::io::BufReader::new(file);
        let data = serde_json::from_reader(file).in_current_span()?;
        Ok(Self { cache_path: Some(path), data })
    }

    /// Returns the [MemDb] it holds access to
    pub fn db(&self) -> &Arc<MemDb> {
        &self.data.data
    }

    /// Metadata stored alongside the data
    pub fn meta(&self) -> &Arc<RwLock<BlockchainDbMeta>> {
        &self.data.meta
    }

    /// Returns `true` if this is a transient cache and nothing will be flushed
    pub fn is_transient(&self) -> bool {
        self.cache_path.is_none()
    }

    /// Flushes the DB to disk if caching is enabled
    pub fn flush(&self) {
        // writes the data to a json file
        if let Some(ref path) = self.cache_path {
            trace!(target: "cache", "saving json cache path={:?}", path);
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::File::create(path)
                .map_err(|e| warn!(target: "cache", "Failed to open json cache for writing: {}", e))
                .and_then(|f| {
                    serde_json::to_writer(BufWriter::new(f), &self.data)
                        .map_err(|e| warn!(target: "cache" ,"Failed to write to json cache: {}", e))
                });
        }
    }
}

/// The Data the [JsonBlockCacheDB] can read and flush
///
/// This will be deserialized in a JSON object with the keys:
/// `["meta", "accounts", "storage", "block_hashes"]`
#[derive(Debug)]
pub struct JsonBlockCacheData {
    pub meta: Arc<RwLock<BlockchainDbMeta>>,
    pub data: Arc<MemDb>,
}

impl Serialize for JsonBlockCacheData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(4))?;

        let meta = self.meta.read();
        map.serialize_entry("meta", &*meta)?;
        drop(meta);

        let accounts = self.data.accounts.read();
        map.serialize_entry("accounts", &*accounts)?;
        drop(accounts);

        let storage = self.data.storage.read();
        map.serialize_entry("storage", &*storage)?;
        drop(storage);

        let block_hashes = self.data.block_hashes.read();
        map.serialize_entry("block_hashes", &*block_hashes)?;
        drop(block_hashes);

        map.end()
    }
}

impl<'de> Deserialize<'de> for JsonBlockCacheData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Data {
            meta: BlockchainDbMeta,
            accounts: BTreeMap<Address, AccountInfo>,
            storage: BTreeMap<Address, StorageInfo>,
            block_hashes: BTreeMap<u64, H256>,
        }

        let Data { meta, accounts, storage, block_hashes } = Data::deserialize(deserializer)?;

        Ok(JsonBlockCacheData {
            meta: Arc::new(RwLock::new(meta)),
            data: Arc::new(MemDb {
                accounts: RwLock::new(accounts),
                storage: RwLock::new(storage),
                block_hashes: RwLock::new(block_hashes),
            }),
        })
    }
}
