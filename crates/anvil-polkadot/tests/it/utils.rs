use alloy_primitives::hex;
use anvil_core::eth::EthRequest;
use anvil_polkadot::{
    api_server::{self, ApiHandle},
    config::{AnvilNodeConfig, SubstrateNodeConfig},
    logging::LoggingManager,
    opts::SubstrateCli,
    spawn,
    substrate_node::service::Service,
};
use anvil_rpc::response::ResponseResult;
use eyre::{Result, WrapErr};
use futures::channel::oneshot;
use parity_scale_codec::Decode;
use polkadot_sdk::{
    sc_cli::CliConfiguration,
    sc_client_api::{BlockBackend, HeaderBackend},
    sp_core::{storage::StorageKey, twox_128, H256},
};
use serde_json::{json, Value};
use tempfile::TempDir;

pub struct TestNode {
    pub service: Service,
    pub api: ApiHandle,
    _temp_dir: Option<TempDir>,
}

impl TestNode {
    pub async fn new(
        anvil_config: AnvilNodeConfig,
        mut substrate_config: SubstrateNodeConfig,
    ) -> Result<Self> {
        let handle = tokio::runtime::Handle::current();

        let mut temp_dir = None;
        match substrate_config
            .base_path()
            .expect("We are in dev mode and failed to create a temp dir")
        {
            None => {
                let temp = tempfile::tempdir().expect("Failed to create temp dir");
                let db_path = temp.path().join("db");
                temp_dir = Some(temp);
                substrate_config.set_base_path(Some(db_path));
            }
            Some(_) if substrate_config.shared_params().is_dev() => {
                let temp = tempfile::tempdir().expect("Failed to create temp dir");
                let db_path = temp.path().join("db");
                temp_dir = Some(temp);
                substrate_config.set_base_path(Some(db_path));
            }
            Some(_) => {}
        }

        let substrate_client = SubstrateCli {};
        let config = substrate_config.create_configuration(&substrate_client, handle.clone())?;
        let (service, api) = spawn(anvil_config, config, LoggingManager::default()).await?;

        Ok(Self { service, api, _temp_dir: temp_dir })
    }

    pub async fn eth_rpc(&mut self, req: EthRequest) -> Result<ResponseResult> {
        let (tx, rx) = oneshot::channel();
        self.api
            .try_send(api_server::ApiRequest { req: req.clone(), resp_sender: tx })
            .map_err(|e| eyre::eyre!("failed to send EthRequest {:?}: {}", req, e))?;

        rx.await.map_err(|e| eyre::eyre!("ApiRequest receiver dropped: {}", e))
    }

    async fn substrate_rpc(&self, method: &str, params: Value) -> Result<Value> {
        let rpc = &self.service.rpc_handlers;

        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });

        let (response, _receiver) = rpc
            .rpc_query(&request.to_string())
            .await
            .wrap_err(format!("RPC call failed for method: {method}"))?;

        let response_value: Value =
            serde_json::from_str(&response).wrap_err("Failed to parse RPC response")?;

        if let Some(error) = response_value.get("error") {
            return Err(eyre::eyre!("RPC error: {}", error));
        }

        response_value
            .get("result")
            .cloned()
            .ok_or_else(|| eyre::eyre!("No result in RPC response"))
    }
}

impl TestNode {
    pub async fn get_best_block_number(&self) -> Result<u32> {
        let best_number = self.service.client.info().best_number;
        Ok(best_number)
    }

    pub async fn block_hash_by_number(&self, n: u32) -> eyre::Result<H256> {
        self.service
            .client
            .block_hash(n)
            .wrap_err("client.block_hash failed")?
            .ok_or_else(|| eyre::eyre!("no hash for block {}", n))
    }

    pub fn create_storage_key(pallet: &str, item: &str) -> StorageKey {
        let mut key = Vec::new();
        key.extend_from_slice(&twox_128(pallet.as_bytes()));
        key.extend_from_slice(&twox_128(item.as_bytes()));
        StorageKey(key)
    }

    pub async fn state_get_storage(
        &self,
        key: StorageKey,
        at: Option<H256>,
    ) -> Result<Option<String>> {
        let key_hex = format!("0x{}", hex::encode(&key.0));
        let result = match at {
            Some(hash) => self.substrate_rpc("state_getStorageAt", json!([key_hex, hash])).await?,
            None => self.substrate_rpc("state_getStorage", json!([key_hex])).await?,
        };

        Ok(result.as_str().map(|s| s.to_string()))
    }

    pub async fn get_decoded_timestamp(&self, at: Option<H256>) -> u64 {
        let storage_key = Self::create_storage_key("Timestamp", "Now");
        let encoded_value = self.state_get_storage(storage_key, at).await.unwrap().unwrap();
        let bytes =
            hex::decode(encoded_value.strip_prefix("0x").unwrap_or(&encoded_value)).unwrap();
        let mut input = &bytes[..];
        Decode::decode(&mut input).unwrap()
    }
}
