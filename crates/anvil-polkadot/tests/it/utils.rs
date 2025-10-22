use alloy_eips::BlockId;
use alloy_primitives::{Address, B256, Bytes, U256, hex};
use alloy_rpc_types::{TransactionInput, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil_core::eth::EthRequest;
use anvil_polkadot::{
    api_server::{
        self, ApiHandle,
        revive_conversions::{AlloyU256, ReviveAddress},
    },
    config::{AnvilNodeConfig, SubstrateNodeConfig},
    init_tracing,
    logging::LoggingManager,
    opts::SubstrateCli,
    spawn,
    substrate_node::{
        genesis::GenesisConfig,
        service::{Service, storage::well_known_keys},
    },
};
use anvil_rpc::{
    error::{ErrorCode, RpcError},
    response::ResponseResult,
};
use codec::Decode;
use eyre::{Result, WrapErr};
use futures::{StreamExt, channel::oneshot};
use polkadot_sdk::{
    pallet_revive::evm::{Block, HashesOrTransactionInfos, ReceiptInfo},
    polkadot_sdk_frame::traits::Header,
    sc_cli::CliConfiguration,
    sc_client_api::{BlockBackend, BlockchainEvents},
    sc_service::TaskManager,
    sp_core::H256,
    sp_state_machine::StorageKey,
};
use serde_json::{Value, json};
use std::{fmt::Debug, time::Duration};
use subxt::utils::H160;
use tempfile::TempDir;

const NATIVE_TO_ETH_RATIO: u128 = 1000000;
pub const EXISTENTIAL_DEPOSIT: u128 = substrate_runtime::currency::DOLLARS * NATIVE_TO_ETH_RATIO;

pub struct BlockWaitTimeout {
    pub block_number: u32,
    pub timeout: Duration,
}

impl BlockWaitTimeout {
    pub fn new(block_number: u32, timeout: Duration) -> Self {
        Self { block_number, timeout }
    }
}

pub struct TestNode {
    pub service: Service,
    pub api: ApiHandle,
    _temp_dir: Option<TempDir>,
    _task_manager: TaskManager,
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

        let substrate_client = SubstrateCli { genesis_config: GenesisConfig::from(&anvil_config) };
        let config = substrate_config.create_configuration(&substrate_client, handle.clone())?;
        let logging_manager = if anvil_config.enable_tracing {
            init_tracing(anvil_config.silent)
        } else {
            LoggingManager::default()
        };

        let (service, task_manager, api) = spawn(anvil_config, config, logging_manager).await?;

        Ok(Self { service, api, _temp_dir: temp_dir, _task_manager: task_manager })
    }

    pub async fn eth_rpc(&mut self, req: EthRequest) -> Result<ResponseResult> {
        let (tx, rx) = oneshot::channel();
        self.api
            .try_send(api_server::ApiRequest { req: req.clone(), resp_sender: tx })
            .map_err(|e| eyre::eyre!("failed to send EthRequest {:?}: {}", req, e))?;

        rx.await.map_err(|e| eyre::eyre!("ApiRequest receiver dropped: {}", e))
    }

    pub async fn block_hash_by_number(&self, n: u32) -> eyre::Result<H256> {
        self.service
            .client
            .block_hash(n)
            .wrap_err("client.block_hash failed")?
            .ok_or_else(|| eyre::eyre!("no hash for block {}", n))
    }

    /// Execute an ethereum transaction.
    pub async fn send_transaction(
        &mut self,
        transaction: TransactionRequest,
        timeout: Option<BlockWaitTimeout>,
    ) -> Result<H256, RpcError> {
        self.send_transaction_inner(transaction, timeout, false).await
    }

    /// Execute an impersonated ethereum transaction.
    pub async fn send_unsigned_transaction(
        &mut self,
        transaction: TransactionRequest,
        timeout: Option<BlockWaitTimeout>,
    ) -> Result<H256, RpcError> {
        self.send_transaction_inner(transaction, timeout, true).await
    }

    async fn send_transaction_inner(
        &mut self,
        transaction: TransactionRequest,
        timeout: Option<BlockWaitTimeout>,
        unsigned: bool,
    ) -> Result<H256, RpcError> {
        let tx_hash = if unsigned {
            unwrap_response::<H256>(
                self.eth_rpc(EthRequest::EthSendUnsignedTransaction(Box::new(
                    WithOtherFields::new(transaction),
                )))
                .await
                .unwrap(),
            )?
        } else {
            unwrap_response::<H256>(
                self.eth_rpc(EthRequest::EthSendTransaction(Box::new(WithOtherFields::new(
                    transaction,
                ))))
                .await
                .unwrap(),
            )?
        };

        if let Some(BlockWaitTimeout { block_number, timeout }) = timeout {
            self.wait_for_block_with_timeout(block_number, timeout).await.unwrap();
        }
        Ok(tx_hash)
    }

    pub async fn get_decoded_timestamp(&self, at: Option<H256>) -> u64 {
        let encoded_value =
            self.state_get_storage(well_known_keys::TIMESTAMP.to_vec(), at).await.unwrap().unwrap();
        let bytes =
            hex::decode(encoded_value.strip_prefix("0x").unwrap_or(&encoded_value)).unwrap();
        let mut input = &bytes[..];
        Decode::decode(&mut input).unwrap()
    }

    pub async fn get_nonce(&mut self, address: Address) -> U256 {
        unwrap_response::<U256>(
            self.eth_rpc(EthRequest::EthGetTransactionCount(address, None)).await.unwrap(),
        )
        .unwrap()
    }

    pub async fn best_block_number(&self) -> u32 {
        let num = self
            .substrate_rpc("chain_getHeader", json!([]))
            .await
            .unwrap()
            .get("number")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_owned();
        u32::from_str_radix(num.trim_start_matches("0x"), 16).unwrap()
    }

    pub async fn wait_for_block_with_timeout(
        &self,
        n: u32,
        timeout: std::time::Duration,
    ) -> eyre::Result<()> {
        if n <= self.best_block_number().await {
            return Ok(());
        }
        tokio::time::timeout(timeout, self.wait_for_block_with_number(n))
            .await
            .map_err(|e| e.into())
    }

    pub async fn get_balance(&mut self, address: H160, block: Option<BlockId>) -> U256 {
        unwrap_response::<U256>(
            self.eth_rpc(EthRequest::EthGetBalance(
                Address::from(ReviveAddress::new(address)),
                block,
            ))
            .await
            .unwrap(),
        )
        .unwrap()
    }

    pub async fn get_transaction_receipt(&mut self, tx_hash: H256) -> ReceiptInfo {
        unwrap_response::<Option<ReceiptInfo>>(
            self.eth_rpc(EthRequest::EthGetTransactionReceipt(B256::from(
                tx_hash.to_fixed_bytes(),
            )))
            .await
            .unwrap(),
        )
        .unwrap()
        .unwrap()
    }

    pub async fn get_block_by_hash(&mut self, hash: H256) -> Block {
        unwrap_response::<Block>(
            self.eth_rpc(EthRequest::EthGetBlockByHash(hash.as_fixed_bytes().into(), false))
                .await
                .unwrap(),
        )
        .unwrap()
    }

    // Initialize with some balance a random account and return its address.
    //
    // Returns the initialized random account address and transaction hash.
    // When a block wait time is provided, it is assumed that automine was
    // previously enabled on the node.
    pub async fn eth_transfer_to_unitialized_random_account(
        &mut self,
        from: Address,
        transfer_amount: U256,
        block_wait_timeout: Option<BlockWaitTimeout>,
    ) -> (Address, H256) {
        let dest_addr = Address::random();
        let dest_h160 = H160::from_slice(dest_addr.as_slice());
        let from_h160 = H160::from_slice(from.as_slice());

        // Create a random account with some balance.
        let from_initial_balance = self.get_balance(from_h160, None).await;
        let dest_initial_balance = self.get_balance(dest_h160, None).await;
        assert_eq!(dest_initial_balance, U256::ZERO);

        let transaction =
            TransactionRequest::default().value(transfer_amount).from(from).to(dest_addr);
        let tx_hash = self.send_transaction(transaction, block_wait_timeout).await.unwrap();

        let is_automine =
            unwrap_response::<bool>(self.eth_rpc(EthRequest::GetAutoMine(())).await.unwrap())
                .unwrap();
        if is_automine {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let receipt_info = self.get_transaction_receipt(tx_hash).await;

            // Assert on balances after first transfer.
            let from_balance = self.get_balance(from_h160, None).await;
            let dest_balance = self.get_balance(dest_h160, None).await;
            assert_eq!(
                from_balance,
                from_initial_balance
                    - AlloyU256::from(receipt_info.effective_gas_price * receipt_info.gas_used)
                        .inner()
                    - transfer_amount
                    - U256::from(EXISTENTIAL_DEPOSIT),
                "signer's balance should have changed"
            );
            assert_eq!(
                dest_balance,
                dest_initial_balance + transfer_amount,
                "dest's balance should have changed"
            );
        }

        (dest_addr, tx_hash)
    }

    pub async fn deploy_contract(
        &mut self,
        code: &[u8],
        deployer: H160,
        block_number: Option<u32>,
    ) -> H256 {
        let deploy_contract_tx = TransactionRequest::default()
            .from(Address::from(ReviveAddress::new(deployer)))
            .input(TransactionInput::both(Bytes::copy_from_slice(code)));
        let block_wait = block_number.map(|bn| BlockWaitTimeout {
            block_number: bn,
            timeout: std::time::Duration::from_millis(1000),
        });
        self.send_transaction(deploy_contract_tx, block_wait).await.unwrap()
    }

    pub async fn get_storage_at(&mut self, storage_key: U256, contract_address: H160) -> U256 {
        let result = self
            .eth_rpc(EthRequest::EthGetStorageAt(
                Address::from(ReviveAddress::new(contract_address)),
                storage_key,
                None,
            ))
            .await
            .unwrap();
        let hex_string = unwrap_response::<String>(result).unwrap();
        let hex_value = hex_string.strip_prefix("0x").unwrap_or(&hex_string);
        U256::from_str_radix(hex_value, 16).unwrap()
    }

    async fn wait_for_block_with_number(&self, n: u32) {
        let mut import_stream = self.service.client.import_notification_stream();

        while let Some(notification) = import_stream.next().await {
            let block_number = *notification.header.number();
            if block_number >= n {
                break;
            }
        }
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

    async fn state_get_storage(&self, key: StorageKey, at: Option<H256>) -> Result<Option<String>> {
        let key_hex = format!("0x{}", hex::encode(&key));
        let result = match at {
            Some(hash) => self.substrate_rpc("state_getStorageAt", json!([key_hex, hash])).await?,
            None => self.substrate_rpc("state_getStorage", json!([key_hex])).await?,
        };
        Ok(result.as_str().map(|s| s.to_string()))
    }
}

pub fn is_transaction_in_block(transactions: &HashesOrTransactionInfos, transaction: H256) -> bool {
    if let HashesOrTransactionInfos::Hashes(transactions) = transactions {
        return transactions.contains(&transaction);
    }
    false
}

pub fn assert_with_tolerance<T>(actual: T, expected: T, tolerance: T, message: &str)
where
    T: PartialOrd + std::ops::Sub<Output = T> + Debug + Copy,
{
    let diff = if actual > expected { actual - expected } else { expected - actual };

    if diff > tolerance {
        panic!(
            "{message}\nExpected: {expected:?} ± {tolerance:?}\nActual: {actual:?}\nDifference: {diff:?}",
        );
    }
}

pub fn unwrap_response<T>(response: ResponseResult) -> Result<T, RpcError>
where
    T: serde::de::DeserializeOwned,
{
    match response {
        ResponseResult::Success(value) => serde_json::from_value(value.clone())
            .or_else(|_| serde_json::from_str(&serde_json::to_string(&value)?))
            .map_err(|_| RpcError::new(ErrorCode::ParseError)),
        ResponseResult::Error(err) => Err(err),
    }
}
pub struct ContractCode {
    pub init: Vec<u8>,
    pub runtime: Option<Vec<u8>>,
}

pub fn get_contract_code(name: &str) -> ContractCode {
    let contract_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("test-data/{name}.json"));

    let contract_json: Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(contract_path).unwrap(),
    ))
    .unwrap();

    let init = hex::decode(contract_json.get("bin").unwrap().as_str().unwrap()).unwrap();
    let runtime =
        contract_json.get("bin-runtime").map(|code| hex::decode(code.as_str().unwrap()).unwrap());

    ContractCode { init, runtime }
}

pub fn to_hex_string(value: u64) -> String {
    let hex = hex::encode(value.to_be_bytes());
    let trimmed = hex.trim_start_matches('0');
    let result = if trimmed.is_empty() { "0" } else { trimmed };
    format!("0x{result}")
}
