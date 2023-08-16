//! Cache related abstraction
use crate::executor::backend::snapshot::StateSnapshot;
use hashbrown::HashMap as Map;
use parking_lot::RwLock;
use revm::{
<<<<<<< HEAD
    primitives::{
        Account, AccountInfo, HashMap as Map, B160, B256, KECCAK_EMPTY, U256,
    },
=======
    primitives::{Account, AccountInfo, B160, B256, KECCAK_EMPTY, U256},
>>>>>>> master
    DatabaseCommit,
};
use serde::{ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::Arc,
};

use url::Url;

pub type StorageInfo = Map<U256, U256>;

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
        Self::new_db(meta, cache_path, false)
    }

    /// Creates a new instance of the [BlockchainDb] and skips check when comparing meta
    /// This is useful for offline-start mode when we don't want to fetch metadata of `block`.
    ///
    /// if a `cache_path` is provided it attempts to load a previously stored [JsonBlockCacheData]
    /// and will try to use the cached entries it holds.
    ///
    /// This will return a new and empty [MemDb] if
    ///   - `cache_path` is `None`
    ///   - the file the `cache_path` points to, does not exist
    ///   - the file contains malformed data, or if it couldn't be read
    ///   - the provided `meta` differs from [BlockchainDbMeta] that's stored on disk
    pub fn new_skip_check(meta: BlockchainDbMeta, cache_path: Option<PathBuf>) -> Self {
        Self::new_db(meta, cache_path, true)
    }

    fn new_db(meta: BlockchainDbMeta, cache_path: Option<PathBuf>, skip_check: bool) -> Self {
        trace!(target : "forge::cache", cache=?cache_path, "initialising blockchain db");
        // read cache and check if metadata matches
        let cache = cache_path
            .as_ref()
            .and_then(|p| {
                JsonBlockCacheDB::load(p).ok().filter(|cache| {
                    if skip_check {
                        return true
                    }
                    let mut existing = cache.meta().write();
                    existing.hosts.extend(meta.hosts.clone());
                    if meta != *existing {
                        warn!(target : "cache", "non-matching block metadata");
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
    pub fn accounts(&self) -> &RwLock<Map<B160, AccountInfo>> {
        &self.db.accounts
    }

    /// Returns the map that holds the storage related info
    pub fn storage(&self) -> &RwLock<Map<B160, StorageInfo>> {
        &self.db.storage
    }

    /// Returns the map that holds all the block hashes
    pub fn block_hashes(&self) -> &RwLock<Map<U256, B256>> {
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

    /// Returns the underlying storage
    pub fn db(&self) -> &Arc<MemDb> {
        &self.db
    }
}

/// relevant identifying markers in the context of [BlockchainDb]
#[derive(Debug, Clone, Eq, Serialize)]
pub struct BlockchainDbMeta {
    pub cfg_env: revm::primitives::CfgEnv,
    pub block_env: revm::primitives::BlockEnv,
    /// all the hosts used to connect to
    pub hosts: BTreeSet<String>,
}

impl BlockchainDbMeta {
    /// Creates a new instance
    pub fn new(env: revm::primitives::Env, url: String) -> Self {
        let host = Url::parse(&url)
            .ok()
            .and_then(|url| url.host().map(|host| host.to_string()))
            .unwrap_or(url);

        BlockchainDbMeta {
            cfg_env: env.cfg.clone(),
            block_env: env.block,
            hosts: BTreeSet::from([host]),
        }
    }
}

// ignore hosts to not invalidate the cache when different endpoints are used, as it's commonly the
// case for http vs ws endpoints
impl PartialEq for BlockchainDbMeta {
    fn eq(&self, other: &Self) -> bool {
        self.cfg_env == other.cfg_env && self.block_env == other.block_env
    }
}

impl<'de> Deserialize<'de> for BlockchainDbMeta {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        /// A backwards compatible representation of [revm::CfgEnv]
        ///
        /// This prevents deserialization errors of cache files caused by breaking changes to the
        /// default [revm::CfgEnv], for example enabling an optional feature.
        /// By hand rolling deserialize impl we can prevent cache file issues
        struct CfgEnvBackwardsCompat {
            inner: revm::primitives::CfgEnv,
        }

        impl<'de> Deserialize<'de> for CfgEnvBackwardsCompat {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let mut value = serde_json::Value::deserialize(deserializer)?;

                // we check for breaking changes here
                if let Some(obj) = value.as_object_mut() {
                    // additional field `disable_eip3607` enabled by the `optional_eip3607` feature
                    let key = "disable_eip3607";
                    if !obj.contains_key(key) {
                        obj.insert(key.to_string(), true.into());
                    }
                    // additional field `disable_block_gas_limit` enabled by the
                    // `optional_block_gas_limit` feature
                    let key = "disable_block_gas_limit";
                    if !obj.contains_key(key) {
                        // keep default value
                        obj.insert(key.to_string(), false.into());
                    }
                    let key = "disable_base_fee";
                    if !obj.contains_key(key) {
                        // keep default value
                        obj.insert(key.to_string(), false.into());
                    }
                }

                let cfg_env: revm::primitives::CfgEnv =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(Self { inner: cfg_env })
            }
        }

        // custom deserialize impl to not break existing cache files
        #[derive(Deserialize)]
        struct Meta {
            cfg_env: CfgEnvBackwardsCompat,
            block_env: revm::primitives::BlockEnv,
            /// all the hosts used to connect to
            #[serde(alias = "host")]
            hosts: Hosts,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Hosts {
            Multi(BTreeSet<String>),
            Single(String),
        }

        let Meta { cfg_env, block_env, hosts } = Meta::deserialize(deserializer)?;
        Ok(Self {
            cfg_env: cfg_env.inner,
            block_env,
            hosts: match hosts {
                Hosts::Multi(hosts) => hosts,
                Hosts::Single(host) => BTreeSet::from([host]),
            },
        })
    }
}

/// In Memory cache containing all fetched accounts and storage slots
/// and their values from RPC
#[derive(Debug, Default)]
pub struct MemDb {
    /// Account related data
    pub accounts: RwLock<Map<B160, AccountInfo>>,
    /// Storage related data
    pub storage: RwLock<Map<B160, StorageInfo>>,
    /// All retrieved block hashes
    pub block_hashes: RwLock<Map<U256, B256>>,
}

impl MemDb {
    /// Clears all data stored in this db
    pub fn clear(&self) {
        self.accounts.write().clear();
        self.storage.write().clear();
        self.block_hashes.write().clear();
    }

    // Inserts the account, replacing it if it exists already
    pub fn do_insert_account(&self, address: B160, account: AccountInfo) {
        self.accounts.write().insert(address, account);
    }

    /// The implementation of [DatabaseCommit::commit()]
    pub fn do_commit(&self, changes: Map<B160, Account>) {
        let mut storage = self.storage.write();
        let mut accounts = self.accounts.write();
        for (add, mut acc) in changes {
            if !acc.is_touched() {
                continue
            }
            if acc.is_empty() || acc.is_destroyed {
                accounts.remove(&add);
                storage.remove(&add);
            } else {
                let is_newly_created = acc.is_created();
                // insert account
                if let Some(code_hash) =
                    acc.info.code.as_ref().filter(|code| !code.is_empty()).map(|code| code.hash)
                {
                    acc.info.code_hash = code_hash;
                } else if acc.info.code_hash.is_zero() {
                    acc.info.code_hash = KECCAK_EMPTY;
                }
                accounts.insert(add, acc.info.clone());

                let acc_storage = storage.entry(add).or_default();
                if is_newly_created {
                    acc_storage.clear();
                }
                for (index, value) in acc.storage {
                    if value.present_value() == U256::from(0) {
                        acc_storage.remove(&index);
                    } else {
                        acc_storage.insert(index, value.present_value());
                    }
                }
                if acc_storage.is_empty() {
                    storage.remove(&add);
                }
            }
        }
    }
}

impl Clone for MemDb {
    fn clone(&self) -> Self {
        Self {
            storage: RwLock::new(self.storage.read().clone()),
            accounts: RwLock::new(self.accounts.read().clone()),
            block_hashes: RwLock::new(self.block_hashes.read().clone()),
        }
    }
}

impl DatabaseCommit for MemDb {
    fn commit(&mut self, changes: Map<B160, Account>) {
        self.do_commit(changes)
    }
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
        trace!(target : "cache", ?path, "reading json cache");
        let contents = std::fs::read_to_string(&path).map_err(|err| {
            warn!(?err, ?path, "Failed to read cache file");
            err
        })?;
        let data = serde_json::from_str(&contents).map_err(|err| {
            warn!(target : "cache", ?err, ?path, "Failed to deserialize cache data");
            err
        })?;
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

    /// Flushes the DB to disk if caching is enabled.
    #[tracing::instrument(level = "warn", skip_all, fields(path = ?self.cache_path))]
    pub fn flush(&self) {
        let Some(path) = &self.cache_path else { return };
        trace!(target: "cache", "saving json cache");

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let file = match fs::File::create(path) {
            Ok(file) => file,
            Err(e) => return warn!(target: "cache", %e, "Failed to open json cache for writing"),
        };

        let mut writer = BufWriter::new(file);
        if let Err(e) = serde_json::to_writer(&mut writer, &self.data) {
            return warn!(target: "cache", %e, "Failed to write to json cache")
        }
        if let Err(e) = writer.flush() {
            return warn!(target: "cache", %e, "Failed to flush to json cache")
        }

        trace!(target: "cache", "saved json cache");
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
            #[serde(flatten)]
            data: StateSnapshot,
        }

        let Data { meta, data: StateSnapshot { accounts, storage, block_hashes } } =
            Data::deserialize(deserializer)?;

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

/// A type that flushes a `JsonBlockCacheDB` on drop
///
/// This type intentionally does not implement `Clone` since it's intended that there's only once
/// instance that will flush the cache.
#[derive(Debug)]
pub struct FlushJsonBlockCacheDB(pub Arc<JsonBlockCacheDB>);

impl Drop for FlushJsonBlockCacheDB {
    fn drop(&mut self) {
        trace!(target: "fork::cache", "flushing cache");
        self.0.flush();
        trace!(target: "fork::cache", "flushed cache");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_deserialize_cache() {
        let s = r#"{
    "meta": {
        "cfg_env": {
            "chain_id": "0x539",
            "spec_id": "LATEST",
            "perf_all_precompiles_have_balance": false,
            "perf_analyse_created_bytecodes": "Analyse",
            "limit_contract_code_size": 18446744073709551615,
            "memory_limit": 4294967295
        },
        "block_env": {
            "number": "0xed3ddf",
            "coinbase": "0x0000000000000000000000000000000000000000",
            "timestamp": "0x6324bc3f",
            "difficulty": "0x0",
            "basefee": "0x2e5fda223",
            "gas_limit": "0x1c9c380"
        },
        "hosts": [
            "eth-mainnet.alchemyapi.io"
        ]
    },
    "accounts": {
        "0xb8ffc3cd6e7cf5a098a1c92f48009765b24088dc": {
            "balance": "0x0",
            "nonce": 10,
            "code_hash": "0x3ac64c95eedf82e5d821696a12daac0e1b22c8ee18a9fd688b00cfaf14550aad",
            "code": {
                "bytecode": "0x60806040526004361061006c5763ffffffff7c01000000000000000000000000000000000000000000000000000000006000350416634555d5c9811461012b5780634558850c1461015257806348a0c8dd146101965780635c60da1b146101bf57806386070cfe146101d4575b6127107f665fd576fbbe6f247aff98f5c94a561e3f71ec2d3c988d56f12d342396c50cea6000825a10156100e15760003411361583541616156100dc576040513381523460208201527f15eeaa57c7bd188c1388020bcadc2c436ec60d647d36ef5b9eb3c742217ddee1604082a1005b600080fd5b6100e96101e9565b9050610126816000368080601f0160208091040260200160405190810160405280939291908181526020018383808284375061026c945050505050565b505050005b34801561013757600080fd5b506101406102ad565b60408051918252519081900360200190f35b34801561015e57600080fd5b5061016d6004356024356102b2565b6040805173ffffffffffffffffffffffffffffffffffffffff9092168252519081900360200190f35b3480156101a257600080fd5b506101ab6102e2565b604080519115158252519081900360200190f35b3480156101cb57600080fd5b5061016d6101e9565b3480156101e057600080fd5b50610140610312565b7f3b4bf6bf3ad5000ecf0f989d5befde585c6860fea3e574a4fab4c49d1c177d9c6000527fc67454ed56db7ff90a4bb32fc9a8de1ab3174b221e5fecea22b7503a3111791f6020527f8e2ed18767e9c33b25344c240cdf92034fae56be99e2c07f3d9946d949ffede45473ffffffffffffffffffffffffffffffffffffffff1690565b600061027783610318565b151561028257600080fd5b612710905060008083516020850186855a03f43d604051816000823e8280156102a9578282f35b8282fd5b600290565b600060208181529281526040808220909352908152205473ffffffffffffffffffffffffffffffffffffffff1681565b600061030d7f665fd576fbbe6f247aff98f5c94a561e3f71ec2d3c988d56f12d342396c50cea610352565b905090565b60015481565b60008073ffffffffffffffffffffffffffffffffffffffff83161515610341576000915061034c565b823b90506000811191505b50919050565b54905600a165627a7a72305820968d404e148c1ec7bb58c8df6cbdcaad4978b93a804e00a1f0e97a5e789eacd40029000000000000000000000000000000000000000000000000000000000000000000",
                "hash": "0x3ac64c95eedf82e5d821696a12daac0e1b22c8ee18a9fd688b00cfaf14550aad",
                "state": {
                    "Checked": {
                        "len": 898
                    }
                }
            }
        }
    },
    "storage": {
        "0xa354f35829ae975e850e23e9615b11da1b3dc4de": {
            "0x290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e564": "0x5553444320795661756c74000000000000000000000000000000000000000000",
            "0x10": "0x37fd60ff8346",
            "0x290decd9548b62a8d60345a988386fc84ba6bc95484008f6362f93160ef3e563": "0xb",
            "0x6": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
            "0x5": "0x36ff5b93162e",
            "0x14": "0x29d635a8e000",
            "0x11": "0x63224c73",
            "0x2": "0x6"
        }
    },
    "block_hashes": {
        "0xed3deb": "0xbf7be3174b261ea3c377b6aba4a1e05d5fae7eee7aab5691087c20cf353e9877",
        "0xed3de9": "0xba1c3648e0aee193e7d00dffe4e9a5e420016b4880455641085a4731c1d32eef",
        "0xed3de8": "0x61d1491c03a9295fb13395cca18b17b4fa5c64c6b8e56ee9cc0a70c3f6cf9855",
        "0xed3de7": "0xb54560b5baeccd18350d56a3bee4035432294dc9d2b7e02f157813e1dee3a0be",
        "0xed3dea": "0x816f124480b9661e1631c6ec9ee39350bda79f0cbfc911f925838d88e3d02e4b"
    }
}"#;

        let cache: JsonBlockCacheData = serde_json::from_str(s).unwrap();
        assert_eq!(cache.data.accounts.read().len(), 1);
        assert_eq!(cache.data.storage.read().len(), 1);
        assert_eq!(cache.data.block_hashes.read().len(), 5);

        let _s = serde_json::to_string(&cache).unwrap();
    }
}
