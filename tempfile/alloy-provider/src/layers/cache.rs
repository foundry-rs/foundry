use crate::{ParamsWithBlock, Provider, ProviderCall, ProviderLayer, RootProvider, RpcWithBlock};
use alloy_eips::BlockId;
use alloy_json_rpc::{RpcError, RpcObject, RpcSend};
use alloy_network::Network;
use alloy_primitives::{
    keccak256, Address, BlockHash, Bytes, StorageKey, StorageValue, TxHash, B256, U256,
};
use alloy_rpc_types_eth::{
    BlockNumberOrTag, BlockTransactionsKind, EIP1186AccountProofResponse, Filter, Log,
};
use alloy_transport::{TransportErrorKind, TransportResult};
use lru::LruCache;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{io::BufReader, marker::PhantomData, num::NonZero, path::PathBuf, sync::Arc};

/// A provider layer that caches RPC responses and serves them on subsequent requests.
///
/// In order to initialize the caching layer, the path to the cache file is provided along with the
/// max number of items that are stored in the in-memory LRU cache.
///
/// One can load the cache from the file system by calling `load_cache` and save the cache to the
/// file system by calling `save_cache`.
#[derive(Debug, Clone)]
pub struct CacheLayer {
    /// In-memory LRU cache, mapping requests to responses.
    cache: SharedCache,
}

impl CacheLayer {
    /// Instantiate a new cache layer with the the maximum number of
    /// items to store.
    pub fn new(max_items: u32) -> Self {
        Self { cache: SharedCache::new(max_items) }
    }

    /// Returns the maximum number of items that can be stored in the cache, set at initialization.
    pub const fn max_items(&self) -> u32 {
        self.cache.max_items()
    }

    /// Returns the shared cache.
    pub fn cache(&self) -> SharedCache {
        self.cache.clone()
    }
}

impl<P, N> ProviderLayer<P, N> for CacheLayer
where
    P: Provider<N>,
    N: Network,
{
    type Provider = CacheProvider<P, N>;

    fn layer(&self, inner: P) -> Self::Provider {
        CacheProvider::new(inner, self.cache())
    }
}

/// The [`CacheProvider`] holds the underlying in-memory LRU cache and overrides methods
/// from the [`Provider`] trait. It attempts to fetch from the cache and fallbacks to
/// the RPC in case of a cache miss.
///
/// Most importantly, the [`CacheProvider`] adds `save_cache` and `load_cache` methods
/// to the provider interface, allowing users to save the cache to disk and load it
/// from there on demand.
#[derive(Debug, Clone)]
pub struct CacheProvider<P, N> {
    /// Inner provider.
    inner: P,
    /// In-memory LRU cache, mapping requests to responses.
    cache: SharedCache,
    /// Phantom data
    _pd: PhantomData<N>,
}

impl<P, N> CacheProvider<P, N>
where
    P: Provider<N>,
    N: Network,
{
    /// Instantiate a new cache provider.
    pub const fn new(inner: P, cache: SharedCache) -> Self {
        Self { inner, cache, _pd: PhantomData }
    }
}

/// Uses underlying transport client to fetch data from the RPC.
///
/// This is specific to RPC requests that require the `block_id` parameter.
///
/// Fetches from the RPC and saves the response to the cache.
///
/// Returns a ProviderCall::BoxedFuture
macro_rules! rpc_call_with_block {
    ($cache:expr, $client:expr, $req:expr) => {{
        let client =
            $client.upgrade().ok_or_else(|| TransportErrorKind::custom_str("RPC client dropped"));
        let cache = $cache.clone();
        ProviderCall::BoxedFuture(Box::pin(async move {
            let client = client?;

            let result = client.request($req.method(), $req.params()).map_params(|params| {
                ParamsWithBlock { params, block_id: $req.block_id.unwrap_or(BlockId::latest()) }
            });

            let res = result.await?;
            // Insert into cache.
            let json_str = serde_json::to_string(&res).map_err(TransportErrorKind::custom)?;
            let hash = $req.params_hash()?;
            let _ = cache.put(hash, json_str);

            Ok(res)
        }))
    }};
}

/// Attempts to fetch the response from the cache by using the hash of the request params.
///
/// Fetches from the RPC in case of a cache miss
///
/// This helps overriding [`Provider`] methods that return `RpcWithBlock`.
macro_rules! cache_rpc_call_with_block {
    ($cache:expr, $client:expr, $req:expr) => {{
        if $req.has_block_tag() {
            return rpc_call_with_block!($cache, $client, $req);
        }

        let hash = $req.params_hash().ok();

        if let Some(hash) = hash {
            if let Ok(Some(cached)) = $cache.get_deserialized(&hash) {
                return ProviderCall::BoxedFuture(Box::pin(async move { Ok(cached) }));
            }
        }

        rpc_call_with_block!($cache, $client, $req)
    }};
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<P, N> Provider<N> for CacheProvider<P, N>
where
    P: Provider<N>,
    N: Network,
{
    #[inline(always)]
    fn root(&self) -> &RootProvider<N> {
        self.inner.root()
    }

    async fn get_block_by_hash(
        &self,
        hash: BlockHash,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        let full = match kind {
            BlockTransactionsKind::Full => true,
            BlockTransactionsKind::Hashes => false,
        };

        let req = RequestType::new("eth_getBlockByHash", (hash, full));

        cache_get_or_fetch(&self.cache, req, self.inner.get_block_by_hash(hash, kind)).await
    }

    async fn get_block_by_number(
        &self,
        number: BlockNumberOrTag,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        let full = match kind {
            BlockTransactionsKind::Full => true,
            BlockTransactionsKind::Hashes => false,
        };

        let req = RequestType::new("eth_getBlockByNumber", (number, full));

        cache_get_or_fetch(&self.cache, req, self.inner.get_block_by_number(number, kind)).await
    }

    fn get_block_receipts(
        &self,
        block: BlockId,
    ) -> ProviderCall<(BlockId,), Option<Vec<N::ReceiptResponse>>> {
        let req = RequestType::new("eth_getBlockReceipts", (block,));

        let redirect =
            !matches!(block, BlockId::Hash(_) | BlockId::Number(BlockNumberOrTag::Number(_)));

        if !redirect {
            let params_hash = req.params_hash().ok();

            if let Some(hash) = params_hash {
                if let Ok(Some(cached)) = self.cache.get_deserialized(&hash) {
                    return ProviderCall::BoxedFuture(Box::pin(async move { Ok(cached) }));
                }
            }
        }

        let client = self.inner.weak_client();
        let cache = self.cache.clone();

        ProviderCall::BoxedFuture(Box::pin(async move {
            let client = client
                .upgrade()
                .ok_or_else(|| TransportErrorKind::custom_str("RPC client dropped"))?;

            let result = client.request(req.method(), req.params()).await?;

            let json_str = serde_json::to_string(&result).map_err(TransportErrorKind::custom)?;

            if !redirect {
                let hash = req.params_hash()?;
                let _ = cache.put(hash, json_str);
            }

            Ok(result)
        }))
    }

    fn get_code_at(&self, address: Address) -> RpcWithBlock<Address, Bytes> {
        let client = self.inner.weak_client();
        let cache = self.cache.clone();
        RpcWithBlock::new_provider(move |block_id| {
            let req = RequestType::new("eth_getCode", address).with_block_id(block_id);
            cache_rpc_call_with_block!(cache, client, req)
        })
    }

    async fn get_logs(&self, filter: &Filter) -> TransportResult<Vec<Log>> {
        let req = RequestType::new("eth_getLogs", filter.clone());

        let params_hash = req.params_hash().ok();

        if let Some(hash) = params_hash {
            if let Some(cached) = self.cache.get_deserialized(&hash)? {
                return Ok(cached);
            }
        }

        let result = self.inner.get_logs(filter).await?;

        let json_str = serde_json::to_string(&result).map_err(TransportErrorKind::custom)?;

        let hash = req.params_hash()?;
        let _ = self.cache.put(hash, json_str);

        Ok(result)
    }

    fn get_proof(
        &self,
        address: Address,
        keys: Vec<StorageKey>,
    ) -> RpcWithBlock<(Address, Vec<StorageKey>), EIP1186AccountProofResponse> {
        let client = self.inner.weak_client();
        let cache = self.cache.clone();
        RpcWithBlock::new_provider(move |block_id| {
            let req =
                RequestType::new("eth_getProof", (address, keys.clone())).with_block_id(block_id);
            cache_rpc_call_with_block!(cache, client, req)
        })
    }

    fn get_storage_at(
        &self,
        address: Address,
        key: U256,
    ) -> RpcWithBlock<(Address, U256), StorageValue> {
        let client = self.inner.weak_client();
        let cache = self.cache.clone();
        RpcWithBlock::new_provider(move |block_id| {
            let req = RequestType::new("eth_getStorageAt", (address, key)).with_block_id(block_id);
            cache_rpc_call_with_block!(cache, client, req)
        })
    }

    fn get_transaction_by_hash(
        &self,
        hash: TxHash,
    ) -> ProviderCall<(TxHash,), Option<N::TransactionResponse>> {
        let req = RequestType::new("eth_getTransactionByHash", (hash,));

        let params_hash = req.params_hash().ok();

        if let Some(hash) = params_hash {
            if let Ok(Some(cached)) = self.cache.get_deserialized(&hash) {
                return ProviderCall::BoxedFuture(Box::pin(async move { Ok(cached) }));
            }
        }
        let client = self.inner.weak_client();
        let cache = self.cache.clone();
        ProviderCall::BoxedFuture(Box::pin(async move {
            let client = client
                .upgrade()
                .ok_or_else(|| TransportErrorKind::custom_str("RPC client dropped"))?;
            let result = client.request(req.method(), req.params()).await?;

            let json_str = serde_json::to_string(&result).map_err(TransportErrorKind::custom)?;
            let hash = req.params_hash()?;
            let _ = cache.put(hash, json_str);

            Ok(result)
        }))
    }

    fn get_raw_transaction_by_hash(&self, hash: TxHash) -> ProviderCall<(TxHash,), Option<Bytes>> {
        let req = RequestType::new("eth_getRawTransactionByHash", (hash,));

        let params_hash = req.params_hash().ok();

        if let Some(hash) = params_hash {
            if let Ok(Some(cached)) = self.cache.get_deserialized(&hash) {
                return ProviderCall::BoxedFuture(Box::pin(async move { Ok(cached) }));
            }
        }

        let client = self.inner.weak_client();
        let cache = self.cache.clone();
        ProviderCall::BoxedFuture(Box::pin(async move {
            let client = client
                .upgrade()
                .ok_or_else(|| TransportErrorKind::custom_str("RPC client dropped"))?;

            let result = client.request(req.method(), req.params()).await?;

            let json_str = serde_json::to_string(&result).map_err(TransportErrorKind::custom)?;
            let hash = req.params_hash()?;
            let _ = cache.put(hash, json_str);

            Ok(result)
        }))
    }

    fn get_transaction_receipt(
        &self,
        hash: TxHash,
    ) -> ProviderCall<(TxHash,), Option<N::ReceiptResponse>> {
        let req = RequestType::new("eth_getTransactionReceipt", (hash,));

        let params_hash = req.params_hash().ok();

        if let Some(hash) = params_hash {
            if let Ok(Some(cached)) = self.cache.get_deserialized(&hash) {
                return ProviderCall::BoxedFuture(Box::pin(async move { Ok(cached) }));
            }
        }

        let client = self.inner.weak_client();
        let cache = self.cache.clone();
        ProviderCall::BoxedFuture(Box::pin(async move {
            let client = client
                .upgrade()
                .ok_or_else(|| TransportErrorKind::custom_str("RPC client dropped"))?;

            let result = client.request(req.method(), req.params()).await?;

            let json_str = serde_json::to_string(&result).map_err(TransportErrorKind::custom)?;
            let hash = req.params_hash()?;
            let _ = cache.put(hash, json_str);

            Ok(result)
        }))
    }
}

/// Internal type to handle different types of requests and generating their param hashes.
struct RequestType<Params: RpcSend> {
    method: &'static str,
    params: Params,
    block_id: Option<BlockId>,
}

impl<Params: RpcSend> RequestType<Params> {
    const fn new(method: &'static str, params: Params) -> Self {
        Self { method, params, block_id: None }
    }

    const fn with_block_id(mut self, block_id: BlockId) -> Self {
        self.block_id = Some(block_id);
        self
    }

    fn params_hash(&self) -> TransportResult<B256> {
        // Merge the method + params and hash them.
        let hash = serde_json::to_string(&self.params())
            .map(|p| keccak256(format!("{}{}", self.method(), p).as_bytes()))
            .map_err(RpcError::ser_err)?;

        Ok(hash)
    }

    const fn method(&self) -> &'static str {
        self.method
    }

    fn params(&self) -> Params {
        self.params.clone()
    }

    /// Returns true if the BlockId has been set to a tag value such as "latest", "earliest", or
    /// "pending".
    const fn has_block_tag(&self) -> bool {
        if let Some(block_id) = self.block_id {
            return !matches!(
                block_id,
                BlockId::Hash(_) | BlockId::Number(BlockNumberOrTag::Number(_))
            );
        }
        false
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct FsCacheEntry {
    /// Hash of the request params
    key: B256,
    /// Serialized response to the request from which the hash was computed.
    value: String,
}

/// Shareable cache.
#[derive(Debug, Clone)]
pub struct SharedCache {
    inner: Arc<RwLock<LruCache<B256, String, alloy_primitives::map::FbBuildHasher<32>>>>,
    max_items: NonZero<usize>,
}

impl SharedCache {
    /// Instantiate a new shared cache.
    pub fn new(max_items: u32) -> Self {
        let max_items = NonZero::new(max_items as usize).unwrap_or(NonZero::<usize>::MIN);
        let inner = Arc::new(RwLock::new(LruCache::with_hasher(max_items, Default::default())));
        Self { inner, max_items }
    }

    /// Maximum number of items that can be stored in the cache.
    pub const fn max_items(&self) -> u32 {
        self.max_items.get() as u32
    }

    /// Puts a value into the cache, and returns the old value if it existed.
    pub fn put(&self, key: B256, value: String) -> TransportResult<bool> {
        Ok(self.inner.write().put(key, value).is_some())
    }

    /// Gets a value from the cache, if it exists.
    pub fn get(&self, key: &B256) -> Option<String> {
        // Need to acquire a write guard to change the order of keys in LRU cache.
        self.inner.write().get(key).cloned()
    }

    /// Get deserialized value from the cache.
    pub fn get_deserialized<T>(&self, key: &B256) -> TransportResult<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let Some(cached) = self.get(key) else { return Ok(None) };
        let result = serde_json::from_str(&cached).map_err(TransportErrorKind::custom)?;
        Ok(Some(result))
    }

    /// Saves the cache to a file specified by the path.
    /// If the files does not exist, it creates one.
    /// If the file exists, it overwrites it.
    pub fn save_cache(&self, path: PathBuf) -> TransportResult<()> {
        let entries: Vec<FsCacheEntry> = {
            self.inner
                .read()
                .iter()
                .map(|(key, value)| FsCacheEntry { key: *key, value: value.clone() })
                .collect()
        };
        let file = std::fs::File::create(path).map_err(TransportErrorKind::custom)?;
        serde_json::to_writer(file, &entries).map_err(TransportErrorKind::custom)?;
        Ok(())
    }

    /// Loads the cache from a file specified by the path.
    /// If the file does not exist, it returns without error.
    pub fn load_cache(&self, path: PathBuf) -> TransportResult<()> {
        if !path.exists() {
            return Ok(());
        };
        let file = std::fs::File::open(path).map_err(TransportErrorKind::custom)?;
        let file = BufReader::new(file);
        let entries: Vec<FsCacheEntry> =
            serde_json::from_reader(file).map_err(TransportErrorKind::custom)?;
        let mut cache = self.inner.write();
        for entry in entries {
            cache.put(entry.key, entry.value);
        }

        Ok(())
    }
}

/// Attempts to fetch the response from the cache by using the hash of the
/// request params.
///
/// In case of a cache miss, fetches from the RPC and saves the response to the
/// cache.
///
/// This helps overriding [`Provider`] methods that return [`TransportResult<T>`].
async fn cache_get_or_fetch<Params: RpcSend, Resp: RpcObject>(
    cache: &SharedCache,
    req: RequestType<Params>,
    fetch_fn: impl std::future::Future<Output = TransportResult<Option<Resp>>>,
) -> TransportResult<Option<Resp>> {
    let hash = req.params_hash()?;
    if let Some(cached) = cache.get_deserialized(&hash)? {
        return Ok(Some(cached));
    }

    let result = fetch_fn.await?;
    if let Some(ref data) = result {
        let json_str = serde_json::to_string(data).map_err(TransportErrorKind::custom)?;
        let _ = cache.put(hash, json_str)?;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderBuilder;
    use alloy_network::{AnyNetwork, TransactionBuilder};
    use alloy_node_bindings::{utils::run_with_tempdir, Anvil};
    use alloy_primitives::{bytes, hex, Bytes, FixedBytes};
    use alloy_rpc_types_eth::{BlockId, TransactionRequest};

    #[tokio::test]
    async fn test_get_block() {
        run_with_tempdir("get-block", |dir| async move {
            let cache_layer = CacheLayer::new(100);
            let shared_cache = cache_layer.cache();
            let anvil = Anvil::new().block_time_f64(0.3).spawn();
            let provider = ProviderBuilder::new().layer(cache_layer).on_http(anvil.endpoint_url());

            let path = dir.join("rpc-cache-block.txt");
            shared_cache.load_cache(path.clone()).unwrap();

            let block = provider.get_block(0.into(), BlockTransactionsKind::Full).await.unwrap(); // Received from RPC.
            let block2 = provider.get_block(0.into(), BlockTransactionsKind::Full).await.unwrap(); // Received from cache.
            assert_eq!(block, block2);

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            let latest_block =
                provider.get_block(BlockId::latest(), BlockTransactionsKind::Full).await.unwrap(); // Received from RPC.
            let latest_hash = latest_block.unwrap().header.hash;

            let block3 =
                provider.get_block_by_hash(latest_hash, BlockTransactionsKind::Full).await.unwrap(); // Received from RPC.
            let block4 =
                provider.get_block_by_hash(latest_hash, BlockTransactionsKind::Full).await.unwrap(); // Received from cache.
            assert_eq!(block3, block4);

            shared_cache.save_cache(path).unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn test_get_block_any_network() {
        run_with_tempdir("get-block", |dir| async move {
            let cache_layer = CacheLayer::new(100);
            let shared_cache = cache_layer.cache();
            let anvil = Anvil::new().block_time_f64(0.3).spawn();
            let provider = ProviderBuilder::new()
                .network::<AnyNetwork>()
                .layer(cache_layer)
                .on_http(anvil.endpoint_url());

            let path = dir.join("rpc-cache-block.txt");
            shared_cache.load_cache(path.clone()).unwrap();

            let block = provider.get_block(0.into(), BlockTransactionsKind::Full).await.unwrap(); // Received from RPC.
            let block2 = provider.get_block(0.into(), BlockTransactionsKind::Full).await.unwrap(); // Received from cache.
            assert_eq!(block, block2);

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            let latest_block =
                provider.get_block(BlockId::latest(), BlockTransactionsKind::Full).await.unwrap(); // Received from RPC.
            let latest_hash = latest_block.unwrap().header.hash;

            let block3 =
                provider.get_block_by_hash(latest_hash, BlockTransactionsKind::Full).await.unwrap(); // Received from RPC.
            let block4 =
                provider.get_block_by_hash(latest_hash, BlockTransactionsKind::Full).await.unwrap(); // Received from cache.
            assert_eq!(block3, block4);

            shared_cache.save_cache(path).unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn test_get_proof() {
        run_with_tempdir("get-proof", |dir| async move {
            let cache_layer = CacheLayer::new(100);
            let shared_cache = cache_layer.cache();
            let anvil = Anvil::new().block_time_f64(0.3).spawn();
            let provider = ProviderBuilder::new().layer(cache_layer).on_http(anvil.endpoint_url());

            let from = anvil.addresses()[0];
            let path = dir.join("rpc-cache-proof.txt");

            shared_cache.load_cache(path.clone()).unwrap();

            let calldata: Bytes = "0x6080604052348015600f57600080fd5b506101f28061001f6000396000f3fe608060405234801561001057600080fd5b50600436106100415760003560e01c80633fb5c1cb146100465780638381f58a14610062578063d09de08a14610080575b600080fd5b610060600480360381019061005b91906100ee565b61008a565b005b61006a610094565b604051610077919061012a565b60405180910390f35b61008861009a565b005b8060008190555050565b60005481565b6000808154809291906100ac90610174565b9190505550565b600080fd5b6000819050919050565b6100cb816100b8565b81146100d657600080fd5b50565b6000813590506100e8816100c2565b92915050565b600060208284031215610104576101036100b3565b5b6000610112848285016100d9565b91505092915050565b610124816100b8565b82525050565b600060208201905061013f600083018461011b565b92915050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052601160045260246000fd5b600061017f826100b8565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101b1576101b0610145565b5b60018201905091905056fea264697066735822122067ac0f21f648b0cacd1b7260772852ad4a0f63e2cc174168c51a6887fd5197a964736f6c634300081a0033".parse().unwrap();

            let tx = TransactionRequest::default()
                .with_from(from)
                .with_input(calldata)
                .with_max_fee_per_gas(1_000_000_000)
                .with_max_priority_fee_per_gas(1_000_000)
                .with_gas_limit(1_000_000)
                .with_nonce(0);

            let tx_receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

            let counter_addr = tx_receipt.contract_address.unwrap();

            let keys = vec![
                FixedBytes::with_last_byte(0),
                FixedBytes::with_last_byte(0x1),
                FixedBytes::with_last_byte(0x2),
                FixedBytes::with_last_byte(0x3),
                FixedBytes::with_last_byte(0x4),
            ];

            let proof =
                provider.get_proof(counter_addr, keys.clone()).block_id(1.into()).await.unwrap();
            let proof2 = provider.get_proof(counter_addr, keys).block_id(1.into()).await.unwrap();

            assert_eq!(proof, proof2);

            shared_cache.save_cache(path).unwrap();
        }).await;
    }

    #[tokio::test]
    async fn test_get_tx_by_hash_and_receipt() {
        run_with_tempdir("get-tx-by-hash", |dir| async move {
            let cache_layer = CacheLayer::new(100);
            let shared_cache = cache_layer.cache();
            let anvil = Anvil::new().block_time_f64(0.3).spawn();
            let provider = ProviderBuilder::new()
                .disable_recommended_fillers()
                .layer(cache_layer)
                .on_http(anvil.endpoint_url());

            let path = dir.join("rpc-cache-tx.txt");
            shared_cache.load_cache(path.clone()).unwrap();

            let req = TransactionRequest::default()
                .from(anvil.addresses()[0])
                .to(Address::repeat_byte(5))
                .value(U256::ZERO)
                .input(bytes!("deadbeef").into());

            let tx_hash =
                *provider.send_transaction(req).await.expect("failed to send tx").tx_hash();

            let tx = provider.get_transaction_by_hash(tx_hash).await.unwrap(); // Received from RPC.
            let tx2 = provider.get_transaction_by_hash(tx_hash).await.unwrap(); // Received from cache.
            assert_eq!(tx, tx2);

            let receipt = provider.get_transaction_receipt(tx_hash).await.unwrap(); // Received from RPC.
            let receipt2 = provider.get_transaction_receipt(tx_hash).await.unwrap(); // Received from cache.

            assert_eq!(receipt, receipt2);

            shared_cache.save_cache(path).unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn test_block_receipts() {
        run_with_tempdir("get-block-receipts", |dir| async move {
            let cache_layer = CacheLayer::new(100);
            let shared_cache = cache_layer.cache();
            let anvil = Anvil::new().spawn();
            let provider = ProviderBuilder::new().layer(cache_layer).on_http(anvil.endpoint_url());

            let path = dir.join("rpc-cache-block-receipts.txt");
            shared_cache.load_cache(path.clone()).unwrap();

            // Send txs

            let receipt = provider
                    .send_raw_transaction(
                        // Transfer 1 ETH from default EOA address to the Genesis address.
                        bytes!("f865808477359400825208940000000000000000000000000000000000000000018082f4f5a00505e227c1c636c76fac55795db1a40a4d24840d81b40d2fe0cc85767f6bd202a01e91b437099a8a90234ac5af3cb7ca4fb1432e133f75f9a91678eaf5f487c74b").as_ref()
                    )
                    .await.unwrap().get_receipt().await.unwrap();

            let block_number = receipt.block_number.unwrap();

            let receipts =
                provider.get_block_receipts(block_number.into()).await.unwrap(); // Received from RPC.
            let receipts2 =
                provider.get_block_receipts(block_number.into()).await.unwrap(); // Received from cache.
            assert_eq!(receipts, receipts2);

            assert!(receipts.is_some_and(|r| r[0] == receipt));

            shared_cache.save_cache(path).unwrap();
        })
        .await
    }

    #[tokio::test]
    async fn test_get_code() {
        run_with_tempdir("get-code", |dir| async move {
            let cache_layer = CacheLayer::new(100);
            let shared_cache = cache_layer.cache();
            let provider = ProviderBuilder::default().with_gas_estimation().layer(cache_layer).on_anvil_with_wallet();

            let path = dir.join("rpc-cache-code.txt");
            shared_cache.load_cache(path.clone()).unwrap();

            let bytecode = hex::decode(
                // solc v0.8.26; solc Counter.sol --via-ir --optimize --bin
                "6080806040523460135760df908160198239f35b600080fdfe6080806040526004361015601257600080fd5b60003560e01c9081633fb5c1cb1460925781638381f58a146079575063d09de08a14603c57600080fd5b3460745760003660031901126074576000546000198114605e57600101600055005b634e487b7160e01b600052601160045260246000fd5b600080fd5b3460745760003660031901126074576020906000548152f35b34607457602036600319011260745760043560005500fea2646970667358221220e978270883b7baed10810c4079c941512e93a7ba1cd1108c781d4bc738d9090564736f6c634300081a0033"
            ).unwrap();
            let tx = TransactionRequest::default().with_nonce(0).with_deploy_code(bytecode).with_chain_id(31337);

            let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

            let counter_addr = receipt.contract_address.unwrap();

            let block_id = BlockId::number(receipt.block_number.unwrap());

            let code = provider.get_code_at(counter_addr).block_id(block_id).await.unwrap(); // Received from RPC.
            let code2 = provider.get_code_at(counter_addr).block_id(block_id).await.unwrap(); // Received from cache.
            assert_eq!(code, code2);

            shared_cache.save_cache(path).unwrap();
        })
        .await;
    }
}
