#![doc = include_str!("../README.md")]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    // TODO:
    // missing_docs,
    unreachable_pub,
    rustdoc::all
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use crate::errors::{is_blocked_by_cloudflare_response, is_cloudflare_security_challenge};
use alloy_chains::{Chain, ChainKind, NamedChain};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, B256};
use contract::ContractMetadata;
use errors::EtherscanError;
use reqwest::{header, IntoUrl, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    borrow::Cow,
    io::Write,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub mod account;
pub mod block_number;
pub mod blocks;
pub mod contract;
pub mod errors;
pub mod gas;
pub mod serde_helpers;
pub mod source_tree;
mod transaction;
pub mod units;
pub mod utils;
pub mod verify;

pub(crate) type Result<T, E = EtherscanError> = std::result::Result<T, E>;

/// The Etherscan.io API client.
#[derive(Clone, Debug)]
pub struct Client {
    /// Client that executes HTTP requests
    client: reqwest::Client,
    /// Etherscan API key
    api_key: Option<String>,
    /// Etherscan API endpoint like <https://api(-chain).etherscan.io/api>
    etherscan_api_url: Url,
    /// Etherscan base endpoint like <https://etherscan.io>
    etherscan_url: Url,
    /// Path to where ABI files should be cached
    cache: Option<Cache>,
    /// Chain ID
    chain_id: Option<u64>,
}

impl Client {
    /// Creates a `ClientBuilder` to configure a `Client`.
    ///
    /// This is the same as `ClientBuilder::default()`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use alloy_chains::Chain;
    /// use foundry_block_explorers::Client;
    /// let client = Client::builder()
    ///     .with_api_key("<API KEY>")
    ///     .chain(Chain::mainnet())
    ///     .unwrap()
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Creates a new instance that caches etherscan requests
    pub fn new_cached(
        chain: Chain,
        api_key: impl Into<String>,
        cache_root: Option<PathBuf>,
        cache_ttl: Duration,
    ) -> Result<Self> {
        let mut this = Self::new(chain, api_key)?;
        this.cache = cache_root.map(|root| Cache::new(root, cache_ttl));
        Ok(this)
    }

    /// Create a new client with the correct endpoints based on the chain and provided API key
    pub fn new(chain: Chain, api_key: impl Into<String>) -> Result<Self> {
        Client::builder().with_api_key(api_key).chain(chain)?.build()
    }

    /// Create a new client with the correct endpoints based on the chain and API key
    /// from the default environment variable defined in [`Chain`].
    pub fn new_from_env(chain: Chain) -> Result<Self> {
        let api_key = match chain.kind() {
            ChainKind::Named(named) => match named {
                // Extra aliases
                NamedChain::Fantom | NamedChain::FantomTestnet => std::env::var("FMTSCAN_API_KEY")
                    .or_else(|_| std::env::var("FANTOMSCAN_API_KEY"))
                    .map_err(Into::into),

                // Backwards compatibility, ideally these should return an error.
                NamedChain::Gnosis
                | NamedChain::Chiado
                | NamedChain::Sepolia
                | NamedChain::Rsk
                | NamedChain::Sokol
                | NamedChain::Poa
                | NamedChain::Oasis
                | NamedChain::Emerald
                | NamedChain::EmeraldTestnet
                | NamedChain::Evmos
                | NamedChain::EvmosTestnet => Ok(String::new()),
                NamedChain::AnvilHardhat | NamedChain::Dev => {
                    Err(EtherscanError::LocalNetworksNotSupported)
                }

                _ => named
                    .etherscan_api_key_name()
                    .ok_or_else(|| EtherscanError::ChainNotSupported(chain))
                    .and_then(|key_name| std::env::var(key_name).map_err(Into::into)),
            },
            ChainKind::Id(_) => Err(EtherscanError::ChainNotSupported(chain)),
        }?;
        Self::new(chain, api_key)
    }

    /// Create a new client with the correct endpoints based on the chain and API key
    /// from the default environment variable defined in [`Chain`].
    ///
    /// If the environment variable is not set, create a new client without it.
    pub fn new_from_opt_env(chain: Chain) -> Result<Self> {
        match Self::new_from_env(chain) {
            Ok(client) => Ok(client),
            Err(EtherscanError::EnvVarNotFound(_)) => {
                Self::builder().chain(chain).and_then(|c| c.build())
            }
            Err(e) => Err(e),
        }
    }

    /// Sets the root to the cache dir and the ttl to use
    pub fn set_cache(&mut self, root: impl Into<PathBuf>, ttl: Duration) -> &mut Self {
        self.cache = Some(Cache { root: root.into(), ttl });
        self
    }

    pub fn etherscan_api_url(&self) -> &Url {
        &self.etherscan_api_url
    }

    pub fn etherscan_url(&self) -> &Url {
        &self.etherscan_url
    }

    /// Return the URL for the given block number
    pub fn block_url(&self, block: u64) -> String {
        self.etherscan_url.join(&format!("block/{block}")).unwrap().to_string()
    }

    /// Return the URL for the given address
    pub fn address_url(&self, address: Address) -> String {
        self.etherscan_url.join(&format!("address/{address:?}")).unwrap().to_string()
    }

    /// Return the URL for the given transaction hash
    pub fn transaction_url(&self, tx_hash: B256) -> String {
        self.etherscan_url.join(&format!("tx/{tx_hash:?}")).unwrap().to_string()
    }

    /// Return the URL for the given token hash
    pub fn token_url(&self, token_hash: Address) -> String {
        self.etherscan_url.join(&format!("token/{token_hash:?}")).unwrap().to_string()
    }

    /// Execute an GET request with parameters.
    async fn get_json<T: DeserializeOwned, Q: Serialize>(&self, query: &Q) -> Result<Response<T>> {
        let res = self.get(query).await?;
        self.sanitize_response(res)
    }

    /// Execute a GET request with parameters, without sanity checking the response.
    async fn get<Q: Serialize>(&self, query: &Q) -> Result<String> {
        trace!(target: "etherscan", "GET {}", self.etherscan_api_url);
        let response = self
            .client
            .get(self.etherscan_api_url.clone())
            .header(header::ACCEPT, "application/json")
            .query(query)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }

    /// Execute a POST request with a form.
    async fn post_form<T: DeserializeOwned, F: Serialize>(&self, form: &F) -> Result<Response<T>> {
        let res = self.post(form).await?;
        self.sanitize_response(res)
    }

    /// Execute a POST request with a form, without sanity checking the response.
    async fn post<F: Serialize>(&self, form: &F) -> Result<String> {
        trace!(target: "etherscan", "POST {}", self.etherscan_api_url);
        let response = self
            .client
            .post(self.etherscan_api_url.clone())
            .form(form)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }

    /// Perform sanity checks on a response and deserialize it into a [Response].
    fn sanitize_response<T: DeserializeOwned>(&self, res: impl AsRef<str>) -> Result<Response<T>> {
        let res = res.as_ref();
        let res: ResponseData<T> = serde_json::from_str(res).map_err(|error| {
            error!(target: "etherscan", ?res, "Failed to deserialize response: {}", error);
            if res == "Page not found" {
                EtherscanError::PageNotFound
            } else if is_blocked_by_cloudflare_response(res) {
                EtherscanError::BlockedByCloudflare
            } else if is_cloudflare_security_challenge(res) {
                EtherscanError::CloudFlareSecurityChallenge
            } else {
                EtherscanError::Serde { error, content: res.to_string() }
            }
        })?;

        match res {
            ResponseData::Error { result, message, status } => {
                if let Some(ref result) = result {
                    if result.starts_with("Max rate limit reached") {
                        return Err(EtherscanError::RateLimitExceeded);
                    } else if result.to_lowercase().contains("invalid api key") {
                        return Err(EtherscanError::InvalidApiKey);
                    }
                }
                Err(EtherscanError::ErrorResponse { status, message, result })
            }
            ResponseData::Success(res) => Ok(res),
        }
    }

    fn create_query<T: Serialize>(
        &self,
        module: &'static str,
        action: &'static str,
        other: T,
    ) -> Query<'_, T> {
        Query {
            apikey: self.api_key.as_deref().map(Cow::Borrowed),
            module: Cow::Borrowed(module),
            action: Cow::Borrowed(action),
            chain_id: self.chain_id,
            other,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClientBuilder {
    /// Client that executes HTTP requests
    client: Option<reqwest::Client>,
    /// Etherscan API key
    api_key: Option<String>,
    /// Etherscan API endpoint like <https://api(-chain).etherscan.io/api>
    etherscan_api_url: Option<Url>,
    /// Etherscan base endpoint like <https://etherscan.io>
    etherscan_url: Option<Url>,
    /// Path to where ABI files should be cached
    cache: Option<Cache>,
    /// Chain ID
    chain_id: Option<u64>,
}

// === impl ClientBuilder ===

impl ClientBuilder {
    /// Configures the etherscan url and api url for the given chain
    ///
    /// Note: This method also sets the chain_id for etherscan multichain verification: <https://docs.etherscan.io/contract-verification/multichain-verification>
    ///
    /// # Errors
    ///
    /// Fails if the chain is not supported by etherscan
    pub fn chain(self, chain: Chain) -> Result<Self> {
        fn urls(
            (api, url): (impl IntoUrl, impl IntoUrl),
        ) -> (reqwest::Result<Url>, reqwest::Result<Url>) {
            (api.into_url(), url.into_url())
        }
        let (etherscan_api_url, etherscan_url) = chain
            .named()
            .ok_or_else(|| EtherscanError::ChainNotSupported(chain))?
            .etherscan_urls()
            .map(urls)
            .ok_or_else(|| EtherscanError::ChainNotSupported(chain))?;

        self.with_chain_id(chain).with_api_url(etherscan_api_url?)?.with_url(etherscan_url?)
    }

    /// Configures the etherscan url
    ///
    /// # Errors
    ///
    /// Fails if the `etherscan_url` is not a valid `Url`
    pub fn with_url(mut self, etherscan_url: impl IntoUrl) -> Result<Self> {
        self.etherscan_url = Some(into_url(etherscan_url)?);
        Ok(self)
    }

    /// Configures the `reqwest::Client`
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Configures the etherscan api url
    ///
    /// # Errors
    ///
    /// Fails if the `etherscan_api_url` is not a valid `Url`
    pub fn with_api_url(mut self, etherscan_api_url: impl IntoUrl) -> Result<Self> {
        self.etherscan_api_url = Some(into_url(etherscan_api_url)?);
        Ok(self)
    }

    /// Configures the etherscan api key
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into()).filter(|s| !s.is_empty());
        self
    }

    /// Configures cache for etherscan request
    pub fn with_cache(mut self, cache_root: Option<PathBuf>, cache_ttl: Duration) -> Self {
        self.cache = cache_root.map(|root| Cache::new(root, cache_ttl));
        self
    }

    /// Configures the chain id for etherscan verification: <https://docs.etherscan.io/contract-verification/multichain-verification>
    pub fn with_chain_id(mut self, chain: Chain) -> Self {
        self.chain_id = Some(chain.id());
        self
    }

    /// Returns the chain the client is built on.
    pub fn get_chain(&self) -> Option<Chain> {
        self.chain_id.map(Chain::from_id)
    }

    /// Returns a Client that uses this ClientBuilder configuration.
    ///
    /// # Errors
    ///
    /// If the following required fields are missing:
    ///   - `etherscan_api_url`
    ///   - `etherscan_url`
    pub fn build(self) -> Result<Client> {
        let ClientBuilder { client, api_key, etherscan_api_url, etherscan_url, cache, chain_id } =
            self;

        let client = Client {
            client: client.unwrap_or_default(),
            api_key,
            etherscan_api_url: etherscan_api_url
                .ok_or_else(|| EtherscanError::Builder("etherscan api url".to_string()))?,
            etherscan_url: etherscan_url
                .ok_or_else(|| EtherscanError::Builder("etherscan url".to_string()))?,
            cache,
            chain_id,
        };
        Ok(client)
    }
}

/// A wrapper around an Etherscan cache object with an expiry
/// time for each item.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct CacheEnvelope<T> {
    // The expiry time is the time the cache item was created + the cache TTL.
    // The cache item is considered expired if the current time is greater than the expiry time.
    expiry: u64,
    // The cached data.
    data: T,
}

/// Simple cache for Etherscan requests.
///
/// The cache is stored at the defined `root` with the following structure:
///
/// - $root/abi/$address.json
/// - $root/sources/$address.json
///
/// Each cache item is stored as a JSON file with the following structure:
///
/// - { "expiry": $expiry, "data": $data }
#[derive(Clone, Debug)]
struct Cache {
    // Path to the cache directory root.
    root: PathBuf,
    // Time to live for each cache item.
    ttl: Duration,
}

impl Cache {
    fn new(root: PathBuf, ttl: Duration) -> Self {
        Self { root, ttl }
    }

    fn get_abi(&self, address: Address) -> Option<Option<JsonAbi>> {
        self.get("abi", address)
    }

    fn set_abi(&self, address: Address, abi: Option<&JsonAbi>) {
        self.set("abi", address, abi)
    }

    fn get_source(&self, address: Address) -> Option<Option<ContractMetadata>> {
        self.get("sources", address)
    }

    fn set_source(&self, address: Address, source: Option<&ContractMetadata>) {
        self.set("sources", address, source)
    }

    fn set<T: Serialize>(&self, prefix: &str, address: Address, item: T) {
        // Create the cache directory if it does not exist.
        let path = self.root.join(prefix);
        if std::fs::create_dir_all(&path).is_err() {
            return;
        }

        let path = path.join(format!("{address:?}.json"));
        let writer = std::fs::File::create(path).ok().map(std::io::BufWriter::new);
        if let Some(mut writer) = writer {
            let _ = serde_json::to_writer(
                &mut writer,
                &CacheEnvelope {
                    expiry: SystemTime::now()
                        .checked_add(self.ttl)
                        .expect("cache ttl overflowed")
                        .duration_since(UNIX_EPOCH)
                        .expect("system time is before unix epoch")
                        .as_secs(),
                    data: item,
                },
            );
            let _ = writer.flush();
        }
    }

    fn get<T: DeserializeOwned>(&self, prefix: &str, address: Address) -> Option<T> {
        let path = self.root.join(prefix).join(format!("{address:?}.json"));

        let Ok(contents) = std::fs::read_to_string(path) else {
            return None;
        };

        let Ok(inner) = serde_json::from_str::<CacheEnvelope<T>>(&contents) else {
            return None;
        };

        // Check if the cache item is still valid.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is before unix epoch")
            // Check if the current time is less than the expiry time
            // to determine if the cache item is still valid.
            .lt(&Duration::from_secs(inner.expiry))
            // If the cache item is still valid, return the data.
            // Otherwise, return None.
            .then_some(inner.data)
    }
}

/// The API response type
#[derive(Debug, Clone, Deserialize)]
pub struct Response<T> {
    pub status: String,
    pub message: String,
    pub result: T,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ResponseData<T> {
    Success(Response<T>),
    Error { status: String, message: String, result: Option<String> },
}

/// The type that gets serialized as query
#[derive(Clone, Debug, Serialize)]
struct Query<'a, T: Serialize> {
    #[serde(skip_serializing_if = "Option::is_none")]
    apikey: Option<Cow<'a, str>>,
    module: Cow<'a, str>,
    action: Cow<'a, str>,
    #[serde(rename = "chainId", skip_serializing_if = "Option::is_none")]
    chain_id: Option<u64>,
    #[serde(flatten)]
    other: T,
}

/// This is a hack to work around `IntoUrl`'s sealed private functions, which can't be called
/// normally.
#[inline]
fn into_url(url: impl IntoUrl) -> std::result::Result<Url, reqwest::Error> {
    url.into_url()
}

#[cfg(test)]
mod tests {
    use crate::{Client, EtherscanError, ResponseData};
    use alloy_chains::Chain;
    use alloy_primitives::{Address, B256};

    // <https://github.com/foundry-rs/foundry/issues/4406>
    #[test]
    fn can_parse_block_scout_err() {
        let err = "{\"message\":\"Something went wrong.\",\"result\":null,\"status\":\"0\"}";
        let resp: ResponseData<Address> = serde_json::from_str(err).unwrap();
        assert!(matches!(resp, ResponseData::Error { .. }));
    }

    #[test]
    fn test_api_paths() {
        let client = Client::new(Chain::goerli(), "").unwrap();
        assert_eq!(client.etherscan_api_url.as_str(), "https://api-goerli.etherscan.io/api");

        assert_eq!(client.block_url(100), "https://goerli.etherscan.io/block/100");
    }

    #[test]
    fn stringifies_block_url() {
        let etherscan = Client::new(Chain::mainnet(), "").unwrap();
        let block: u64 = 1;
        let block_url: String = etherscan.block_url(block);
        assert_eq!(block_url, format!("https://etherscan.io/block/{block}"));
    }

    #[test]
    fn stringifies_address_url() {
        let etherscan = Client::new(Chain::mainnet(), "").unwrap();
        let addr: Address = Address::ZERO;
        let address_url: String = etherscan.address_url(addr);
        assert_eq!(address_url, format!("https://etherscan.io/address/{addr:?}"));
    }

    #[test]
    fn stringifies_transaction_url() {
        let etherscan = Client::new(Chain::mainnet(), "").unwrap();
        let tx_hash = B256::ZERO;
        let tx_url: String = etherscan.transaction_url(tx_hash);
        assert_eq!(tx_url, format!("https://etherscan.io/tx/{tx_hash:?}"));
    }

    #[test]
    fn stringifies_token_url() {
        let etherscan = Client::new(Chain::mainnet(), "").unwrap();
        let token_hash = Address::ZERO;
        let token_url: String = etherscan.token_url(token_hash);
        assert_eq!(token_url, format!("https://etherscan.io/token/{token_hash:?}"));
    }

    #[test]
    fn local_networks_not_supported() {
        let err = Client::new_from_env(Chain::dev()).unwrap_err();
        assert!(matches!(err, EtherscanError::LocalNetworksNotSupported));
    }

    #[test]
    fn can_parse_etherscan_mainnet_invalid_api_key() {
        let err = serde_json::json!({
            "status":"0",
            "message":"NOTOK",
            "result":"Missing/Invalid API Key"
        });
        let resp: ResponseData<Address> = serde_json::from_value(err).unwrap();
        assert!(matches!(resp, ResponseData::Error { .. }));
    }
}
