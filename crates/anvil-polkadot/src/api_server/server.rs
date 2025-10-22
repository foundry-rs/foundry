use super::revive_conversions::{ReviveBytes, ReviveFilter};
use crate::{
    api_server::{
        ApiRequest,
        error::{Error, Result, ToRpcResponseResult},
        revive_conversions::{
            AlloyU256, ReviveAddress, ReviveBlockId, ReviveBlockNumberOrTag, SubstrateU256,
            convert_to_generic_transaction,
        },
    },
    logging::LoggingManager,
    macros::node_info,
    substrate_node::{
        impersonation::ImpersonationManager,
        in_mem_rpc::InMemoryRpcClient,
        mining_engine::MiningEngine,
        service::{
            BackendError, BackendWithOverlay, Client, Service,
            storage::{
                AccountType, ByteCodeType, CodeInfo, ContractInfo, ReviveAccountInfo,
                SystemAccountInfo,
            },
        },
        snapshot::{RevertInfo, SnapshotManager},
    },
};
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Address, B256, U64, U256};
use alloy_rpc_types::{Filter, TransactionRequest, anvil::MineOptions};
use alloy_serde::WithOtherFields;
use anvil_core::eth::{EthRequest, Params as MineParams};
use anvil_rpc::response::ResponseResult;
use codec::{Decode, Encode};
use futures::{StreamExt, channel::mpsc};
use pallet_revive_eth_rpc::{
    BlockInfoProvider, EthRpcError, ReceiptExtractor, ReceiptProvider, SubxtBlockInfoProvider,
    client::{Client as EthRpcClient, ClientError, SubscriptionType},
    subxt_client::{self, SrcChainConfig},
};
use polkadot_sdk::{
    pallet_revive::{
        ReviveApi,
        evm::{
            Account, Block, Bytes, FeeHistoryResult, FilterResults, ReceiptInfo, TransactionInfo,
            TransactionSigned,
        },
    },
    parachains_common::{AccountId, Hash, Nonce},
    polkadot_sdk_frame::runtime::types_common::OpaqueBlock,
    sc_client_api::HeaderBackend,
    sc_service::SpawnTaskHandle,
    sp_api::{Metadata, ProvideRuntimeApi},
    sp_arithmetic::Permill,
    sp_blockchain::Info,
    sp_core::{self, Hasher, keccak_256},
    sp_runtime::traits::BlakeTwo256,
};
use sqlx::sqlite::SqlitePoolOptions;
use std::{collections::HashSet, sync::Arc, time::Duration};
use substrate_runtime::Balance;
use subxt::{
    Metadata as SubxtMetadata, OnlineClient, backend::rpc::RpcClient,
    client::RuntimeVersion as SubxtRuntimeVersion, config::substrate::H256,
    ext::subxt_rpcs::LegacyRpcMethods, utils::H160,
};

pub const CLIENT_VERSION: &str = concat!("anvil-polkadot/v", env!("CARGO_PKG_VERSION"));

pub struct Wallet {
    accounts: Vec<Account>,
}

pub struct ApiServer {
    eth_rpc_client: EthRpcClient,
    req_receiver: mpsc::Receiver<ApiRequest>,
    backend: BackendWithOverlay,
    logging_manager: LoggingManager,
    client: Arc<Client>,
    mining_engine: Arc<MiningEngine>,
    block_provider: SubxtBlockInfoProvider,
    wallet: Wallet,
    snapshot_manager: SnapshotManager,
    impersonation_manager: ImpersonationManager,
}

impl ApiServer {
    pub async fn new(
        substrate_service: Service,
        req_receiver: mpsc::Receiver<ApiRequest>,
        logging_manager: LoggingManager,
        snapshot_manager: SnapshotManager,
        impersonation_manager: ImpersonationManager,
    ) -> Result<Self> {
        let rpc_client = RpcClient::new(InMemoryRpcClient(substrate_service.rpc_handlers.clone()));
        let api = create_online_client(&substrate_service, rpc_client.clone()).await?;
        let rpc = LegacyRpcMethods::<SrcChainConfig>::new(rpc_client.clone());
        let block_provider = SubxtBlockInfoProvider::new(api.clone(), rpc.clone()).await?;
        let eth_rpc_client = create_revive_rpc_client(
            api.clone(),
            rpc_client.clone(),
            rpc,
            block_provider.clone(),
            substrate_service.spawn_handle.clone(),
        )
        .await?;

        Ok(Self {
            block_provider,
            req_receiver,
            logging_manager,
            backend: BackendWithOverlay::new(
                substrate_service.backend.clone(),
                substrate_service.storage_overrides.clone(),
            ),
            client: substrate_service.client.clone(),
            mining_engine: substrate_service.mining_engine.clone(),
            eth_rpc_client,
            snapshot_manager,
            impersonation_manager,
            wallet: Wallet {
                accounts: vec![
                    Account::from(subxt_signer::eth::dev::baltathar()),
                    Account::from(subxt_signer::eth::dev::alith()),
                    Account::from(subxt_signer::eth::dev::charleth()),
                ],
            },
        })
    }

    pub async fn run(mut self) {
        while let Some(msg) = self.req_receiver.next().await {
            let resp = self.execute(msg.req).await;

            msg.resp_sender.send(resp).expect("Dropped receiver");
        }
    }

    pub async fn execute(&mut self, req: EthRequest) -> ResponseResult {
        let res = match req.clone() {
            EthRequest::SetLogging(enabled) => self.set_logging(enabled).to_rpc_result(),

            //------- Mining---------
            EthRequest::Mine(blocks, interval) => self.mine(blocks, interval).await.to_rpc_result(),
            EthRequest::SetIntervalMining(interval) => {
                self.set_interval_mining(interval).to_rpc_result()
            }
            EthRequest::GetIntervalMining(_) => self.get_interval_mining().to_rpc_result(),
            EthRequest::GetAutoMine(_) => self.get_auto_mine().to_rpc_result(),
            EthRequest::SetAutomine(enabled) => self.set_auto_mine(enabled).to_rpc_result(),
            EthRequest::EvmMine(mine) => self.evm_mine(mine).await.to_rpc_result(),
            EthRequest::EvmMineDetailed(mine) => self.evm_mine_detailed(mine).await.to_rpc_result(),

            //------- TimeMachine---------
            EthRequest::EvmSetBlockTimeStampInterval(time) => {
                self.set_block_timestamp_interval(time).to_rpc_result()
            }
            EthRequest::EvmRemoveBlockTimeStampInterval(_) => {
                self.remove_block_timestamp_interval().to_rpc_result()
            }
            EthRequest::EvmSetNextBlockTimeStamp(time) => {
                self.set_next_block_timestamp(time).to_rpc_result()
            }
            EthRequest::EvmIncreaseTime(time) => self.increase_time(time).to_rpc_result(),
            EthRequest::EvmSetTime(timestamp) => self.set_time(timestamp).to_rpc_result(),

            // -- Impersonation --
            EthRequest::ImpersonateAccount(addr) => {
                self.impersonate_account(H160::from_slice(addr.0.as_ref())).to_rpc_result()
            }
            EthRequest::StopImpersonatingAccount(addr) => {
                self.stop_impersonating_account(&H160::from_slice(addr.0.as_ref())).to_rpc_result()
            }
            EthRequest::AutoImpersonateAccount(enable) => {
                self.auto_impersonate_account(enable).to_rpc_result()
            }
            EthRequest::EthSendUnsignedTransaction(request) => {
                node_info!("eth_sendUnsignedTransaction");
                self.send_transaction(*request.clone(), true).await.to_rpc_result()
            }

            //------- Eth RPCs---------
            EthRequest::EthChainId(_) => self.eth_chain_id().to_rpc_result(),
            EthRequest::EthNetworkId(_) => self.network_id().to_rpc_result(),
            EthRequest::NetListening(_) => self.net_listening().to_rpc_result(),
            EthRequest::EthSyncing(_) => self.syncing().to_rpc_result(),
            EthRequest::EthGetTransactionReceipt(tx_hash) => {
                self.transaction_receipt(tx_hash).await.to_rpc_result()
            }
            EthRequest::EthGetBalance(addr, block) => {
                self.get_balance(addr, block).await.to_rpc_result()
            }
            EthRequest::EthGetStorageAt(addr, slot, block) => {
                self.get_storage_at(addr, slot, block).await.to_rpc_result()
            }
            EthRequest::EthGetCodeAt(addr, block) => {
                self.get_code(addr, block).await.to_rpc_result()
            }
            EthRequest::EthGetBlockByHash(hash, full) => {
                self.get_block_by_hash(hash, full).await.to_rpc_result()
            }
            EthRequest::EthEstimateGas(call, block, _overrides, _block_overrides) => {
                self.estimate_gas(call, block).await.to_rpc_result()
            }
            EthRequest::EthCall(call, block, _, _) => self.call(call, block).await.to_rpc_result(),
            EthRequest::EthSendTransaction(request) => {
                self.send_transaction(*request.clone(), false).await.to_rpc_result()
            }
            EthRequest::EthGasPrice(()) => self.gas_price().await.to_rpc_result(),
            EthRequest::EthGetBlockByNumber(num, hydrated) => {
                node_info!("eth_getBlockByNumber");
                self.get_block_by_number(num, hydrated).await.to_rpc_result()
            }
            EthRequest::EthGetTransactionCount(addr, block) => self
                .get_transaction_count(ReviveAddress::from(addr).inner(), block)
                .await
                .map(|val| AlloyU256::from(val).inner())
                .to_rpc_result(),

            // --- Snapshot ---
            EthRequest::EvmSnapshot(()) => self.snapshot().await.to_rpc_result(),
            EthRequest::Rollback(depth) => self.rollback(depth).await.to_rpc_result(),
            EthRequest::EvmRevert(id) => self.revert(id).await.to_rpc_result(),

            // ------- State injector ---------
            EthRequest::SetBalance(address, value) => {
                self.set_balance(address, value).to_rpc_result()
            }
            EthRequest::SetNonce(address, value) => self.set_nonce(address, value).to_rpc_result(),
            EthRequest::SetCode(address, bytes) => self.set_code(address, bytes).to_rpc_result(),
            EthRequest::SetStorageAt(address, key, value) => {
                self.set_storage_at(address, key, value).to_rpc_result()
            }
            EthRequest::SetChainId(chain_id) => self.set_chain_id(chain_id).to_rpc_result(),
            EthRequest::EthBlockNumber(()) => {
                node_info!("eth_blockNumber");
                Ok(U256::from(self.client.info().best_number)).to_rpc_result()
            }
            EthRequest::EthGetTransactionCountByHash(hash) => {
                node_info!("eth_getBlockTransactionCountByHash");
                self.get_block_transaction_count_by_hash(hash).await.to_rpc_result()
            }
            EthRequest::EthGetTransactionCountByNumber(num) => {
                node_info!("eth_getBlockTransactionCountByNumber");
                self.get_block_transaction_count_by_number(num).await.to_rpc_result()
            }
            EthRequest::EthGetTransactionByBlockHashAndIndex(hash, index) => {
                node_info!("eth_getTransactionByBlockHashAndIndex");
                self.get_transaction_by_block_hash_and_index(hash, index.into())
                    .await
                    .to_rpc_result()
            }
            EthRequest::EthGetTransactionByBlockNumberAndIndex(num, index) => {
                node_info!("eth_getTransactionByBlockNumberAndIndex");
                self.get_transaction_by_block_number_and_index(num, index.into())
                    .await
                    .to_rpc_result()
            }
            EthRequest::EthGetTransactionByHash(hash) => {
                node_info!("eth_getTransactionByHash");
                self.get_transaction_by_hash(hash).await.to_rpc_result()
            }
            EthRequest::Web3ClientVersion(()) => {
                node_info!("web3_clientVersion");
                Ok(CLIENT_VERSION.to_string()).to_rpc_result()
            }
            EthRequest::EthFeeHistory(count, newest, reward_percentiles) => {
                node_info!("eth_feeHistory");
                self.fee_history(count, newest, Some(reward_percentiles)).await.to_rpc_result()
            }
            EthRequest::EthMaxPriorityFeePerGas(_) => {
                node_info!("eth_maxPriorityFeePerGas");
                self.max_priority_fee_per_gas().await.to_rpc_result()
            }
            EthRequest::EthSendRawTransaction(tx) => {
                node_info!("eth_sendRawTransaction");
                self.send_raw_transaction(ReviveBytes::from(tx).inner()).await.to_rpc_result()
            }
            EthRequest::EthAccounts(_) => {
                node_info!("eth_accounts");
                self.accounts().to_rpc_result()
            }
            EthRequest::EthGetLogs(filter) => {
                node_info!("eth_getLogs");
                self.get_logs(filter).await.to_rpc_result()
            }
            _ => Err::<(), _>(Error::RpcUnimplemented).to_rpc_result(),
        };

        if let ResponseResult::Error(err) = &res {
            node_info!("\nRPC request failed:");
            node_info!("    Request: {:?}", req);
            node_info!("    Error: {}\n", err);
        }

        res
    }

    fn set_logging(&self, enabled: bool) -> Result<()> {
        node_info!("anvil_setLoggingEnabled");

        self.logging_manager.set_enabled(enabled);
        Ok(())
    }

    // Mining related RPCs.
    async fn mine(&self, blocks: Option<U256>, interval: Option<U256>) -> Result<()> {
        node_info!("anvil_mine");

        if blocks.is_some_and(|b| u64::try_from(b).is_err()) {
            return Err(Error::InvalidParams("The number of blocks is too large".to_string()));
        }
        if interval.is_some_and(|i| u64::try_from(i).is_err()) {
            return Err(Error::InvalidParams(
                "The interval between blocks is too large".to_string(),
            ));
        }
        self.mining_engine
            .mine(blocks.map(|b| b.to()), interval.map(|i| Duration::from_secs(i.to())))
            .await
            .map_err(Error::Mining)
    }

    fn set_interval_mining(&self, interval: u64) -> Result<()> {
        node_info!("evm_setIntervalMining");

        self.mining_engine.set_interval_mining(Duration::from_secs(interval));
        Ok(())
    }

    fn get_interval_mining(&self) -> Result<Option<u64>> {
        node_info!("anvil_getIntervalMining");

        Ok(self.mining_engine.get_interval_mining())
    }

    fn get_auto_mine(&self) -> Result<bool> {
        node_info!("anvil_getAutomine");

        Ok(self.mining_engine.is_automine())
    }

    fn set_auto_mine(&self, enabled: bool) -> Result<()> {
        node_info!("evm_setAutomine");

        self.mining_engine.set_auto_mine(enabled);
        Ok(())
    }

    async fn evm_mine(&self, mine: Option<MineParams<Option<MineOptions>>>) -> Result<String> {
        node_info!("evm_mine");

        self.mining_engine.evm_mine(mine.and_then(|p| p.params)).await?;
        Ok("0x0".to_string())
    }

    async fn evm_mine_detailed(
        &self,
        mine: Option<MineParams<Option<MineOptions>>>,
    ) -> Result<Vec<Block>> {
        node_info!("evm_mine_detailed");
        let mined_blocks = self.mining_engine.do_evm_mine(mine.and_then(|p| p.params)).await?;
        let mut blocks = Vec::with_capacity(mined_blocks as usize);
        let last_block = self.client.info().best_number as u64;
        let starting = last_block - mined_blocks + 1;
        for block_number in starting..=last_block {
            if let Some(block) =
                self.get_block_by_number(BlockNumberOrTag::Number(block_number), true).await?
            {
                blocks.push(block);
            }
        }
        Ok(blocks)
    }

    // TimeMachine RPCs
    fn set_block_timestamp_interval(&self, time: u64) -> Result<()> {
        node_info!("anvil_setBlockTimestampInterval");

        self.mining_engine.set_block_timestamp_interval(Duration::from_secs(time));
        Ok(())
    }

    fn remove_block_timestamp_interval(&self) -> Result<bool> {
        node_info!("anvil_removeBlockTimestampInterval");

        Ok(self.mining_engine.remove_block_timestamp_interval())
    }

    fn set_next_block_timestamp(&self, time: U256) -> Result<()> {
        node_info!("anvil_setBlockTimestampInterval");

        if time >= U256::from(u64::MAX) {
            return Err(Error::InvalidParams("The timestamp is too big".to_string()));
        }
        let time = time.to::<u64>();
        self.mining_engine
            .set_next_block_timestamp(Duration::from_secs(time))
            .map_err(Error::Mining)
    }

    fn increase_time(&self, time: U256) -> Result<i64> {
        node_info!("evm_increaseTime");

        Ok(self.mining_engine.increase_time(Duration::from_secs(time.try_into().unwrap_or(0))))
    }

    fn set_time(&self, timestamp: U256) -> Result<u64> {
        node_info!("evm_setTime");

        if timestamp >= U256::from(u64::MAX) {
            return Err(Error::InvalidParams("The timestamp is too big".to_string()));
        }
        let time = timestamp.to::<u64>();
        Ok(self.mining_engine.set_time(Duration::from_secs(time)))
    }

    // Impersonation RPC
    fn impersonate_account(&mut self, addr: H160) -> Result<()> {
        node_info!("anvil_impersonateAccount");
        self.impersonation_manager.impersonate(addr);
        Ok(())
    }

    fn auto_impersonate_account(&mut self, enable: bool) -> Result<()> {
        node_info!("anvil_autoImpersonateAccount");
        self.impersonation_manager.set_auto_impersonate_account(enable);
        Ok(())
    }

    fn stop_impersonating_account(&mut self, addr: &H160) -> Result<()> {
        node_info!("anvil_stopImpersonatingAccount");
        self.impersonation_manager.stop_impersonating(addr);
        Ok(())
    }

    fn chain_id(&self, at: Hash) -> u64 {
        self.backend.read_chain_id(at).expect("Chain ID is populated on genesis")
    }

    // Eth RPCs
    fn eth_chain_id(&self) -> Result<U64> {
        node_info!("eth_chainId");
        let latest_block = self.latest_block();

        Ok(U256::from(self.chain_id(latest_block)).to::<U64>())
    }

    fn network_id(&self) -> Result<u64> {
        node_info!("eth_networkId");
        let latest_block = self.latest_block();

        Ok(self.chain_id(latest_block))
    }

    fn net_listening(&self) -> Result<bool> {
        node_info!("net_listening");
        Ok(true)
    }

    fn syncing(&self) -> Result<bool> {
        node_info!("eth_syncing");
        Ok(false)
    }

    async fn transaction_receipt(&self, tx_hash: B256) -> Result<Option<ReceiptInfo>> {
        node_info!("eth_getTransactionReceipt");
        Ok(self.eth_rpc_client.receipt(&(tx_hash.0.into())).await)
    }

    async fn get_balance(&self, addr: Address, block: Option<BlockId>) -> Result<U256> {
        node_info!("eth_getBalance");
        let hash = self.get_block_hash_for_tag(block).await?;

        let runtime_api = self.eth_rpc_client.runtime_api(hash);
        let balance = runtime_api.balance(ReviveAddress::from(addr).inner()).await?;
        Ok(AlloyU256::from(balance).inner())
    }

    async fn get_storage_at(
        &self,
        addr: Address,
        slot: U256,
        block: Option<BlockId>,
    ) -> Result<Bytes> {
        node_info!("eth_getStorageAt");
        let hash = self.get_block_hash_for_tag(block).await?;
        let runtime_api = self.eth_rpc_client.runtime_api(hash);
        let bytes =
            runtime_api.get_storage(ReviveAddress::from(addr).inner(), slot.to_be_bytes()).await?;
        Ok(bytes.unwrap_or_default().into())
    }

    async fn get_code(&self, address: Address, block: Option<BlockId>) -> Result<Bytes> {
        node_info!("eth_getCode");

        let hash = self.get_block_hash_for_tag(block).await?;
        let code = self
            .eth_rpc_client
            .runtime_api(hash)
            .code(ReviveAddress::from(address).inner())
            .await?;
        Ok(code.into())
    }

    async fn get_block_by_hash(
        &self,
        block_hash: B256,
        hydrated_transactions: bool,
    ) -> Result<Option<Block>> {
        node_info!("eth_getBlockByHash");
        let Some(block) =
            self.eth_rpc_client.block_by_hash(&H256::from_slice(block_hash.as_slice())).await?
        else {
            return Ok(None);
        };
        let block = self.eth_rpc_client.evm_block(block, hydrated_transactions).await;
        Ok(Some(block))
    }

    async fn estimate_gas(
        &self,
        request: WithOtherFields<TransactionRequest>,
        block: Option<alloy_rpc_types::BlockId>,
    ) -> Result<sp_core::U256> {
        node_info!("eth_estimateGas");

        let hash = self.get_block_hash_for_tag(block).await?;
        let runtime_api = self.eth_rpc_client.runtime_api(hash);
        let dry_run =
            runtime_api.dry_run(convert_to_generic_transaction(request.into_inner())).await?;
        Ok(dry_run.eth_gas)
    }

    async fn call(
        &self,
        request: WithOtherFields<TransactionRequest>,
        block: Option<alloy_rpc_types::BlockId>,
    ) -> Result<Bytes> {
        node_info!("eth_call");

        let hash = self.get_block_hash_for_tag(block).await?;
        let runtime_api = self.eth_rpc_client.runtime_api(hash);
        let dry_run =
            runtime_api.dry_run(convert_to_generic_transaction(request.into_inner())).await?;

        Ok(dry_run.data.into())
    }

    async fn gas_price(&self) -> Result<sp_core::U256> {
        node_info!("eth_gasPrice");

        let hash = self.latest_block();

        let runtime_api = self.eth_rpc_client.runtime_api(hash);
        runtime_api.gas_price().await.map_err(Error::from)
    }

    async fn get_transaction_count(
        &self,
        address: H160,
        block: Option<BlockId>,
    ) -> Result<sp_core::U256> {
        node_info!("eth_getTransactionCount");
        let hash = self.get_block_hash_for_tag(block).await?;
        let runtime_api = self.eth_rpc_client.runtime_api(hash);
        let nonce = runtime_api.nonce(address).await?;
        Ok(nonce)
    }

    async fn send_raw_transaction(&self, transaction: Bytes) -> Result<H256> {
        let hash = H256(keccak_256(&transaction.0));
        let call = subxt_client::tx().revive().eth_transact(transaction.0);
        self.eth_rpc_client.submit(call).await?;
        Ok(hash)
    }

    async fn send_transaction(
        &self,
        transaction_req: WithOtherFields<TransactionRequest>,
        unsigned_tx: bool,
    ) -> Result<H256> {
        node_info!("eth_sendTransaction");
        let mut transaction = convert_to_generic_transaction(transaction_req.clone().into_inner());
        let Some(from) = transaction.from else {
            return Err(Error::ReviveRpc(EthRpcError::InvalidTransaction));
        };

        let latest_block = self.latest_block();
        let latest_block_id = Some(BlockId::hash(B256::from_slice(latest_block.as_ref())));

        if transaction.gas.is_none() {
            transaction.gas =
                Some(self.estimate_gas(transaction_req.clone(), latest_block_id).await?);
        }

        if transaction.gas_price.is_none() {
            transaction.gas_price = Some(self.gas_price().await?);
        }
        if transaction.nonce.is_none() {
            transaction.nonce = Some(self.get_transaction_count(from, latest_block_id).await?);
        }
        if transaction.chain_id.is_none() {
            transaction.chain_id =
                Some(sp_core::U256::from_big_endian(&self.chain_id(latest_block).to_be_bytes()));
        }

        let tx = transaction
            .try_into_unsigned()
            .map_err(|_| Error::ReviveRpc(EthRpcError::InvalidTransaction))?;

        let payload = if self.impersonation_manager.is_impersonated(from) || unsigned_tx {
            let mut fake_signature = [0; 65];
            fake_signature[12..32].copy_from_slice(from.as_bytes());
            tx.with_signature(fake_signature).signed_payload()
        } else {
            let account = self
                .wallet
                .accounts
                .iter()
                .find(|account| account.address() == from)
                .ok_or(Error::ReviveRpc(EthRpcError::AccountNotFound(from)))?;
            account.sign_transaction(tx).signed_payload()
        };

        self.send_raw_transaction(Bytes(payload)).await
    }

    async fn get_block_by_number(
        &self,
        block_number: BlockNumberOrTag,
        hydrated_transactions: bool,
    ) -> Result<Option<Block>> {
        let Some(block) = self
            .eth_rpc_client
            .block_by_number_or_tag(&ReviveBlockNumberOrTag::from(block_number).inner())
            .await?
        else {
            return Ok(None);
        };
        let block = self.eth_rpc_client.evm_block(block, hydrated_transactions).await;
        Ok(Some(block))
    }

    pub(crate) async fn snapshot(&mut self) -> Result<U256> {
        node_info!("evm_snapshot");
        Ok(self.snapshot_manager.snapshot())
    }

    pub(crate) async fn revert(&mut self, id: U256) -> Result<bool> {
        node_info!("evm_revert");
        let res = self
            .snapshot_manager
            .revert(id)
            .map_err(|err| Error::Backend(BackendError::Client(err)))?;
        let Some(res) = res else { return Ok(false) };

        self.on_revert_update(res).await?;

        Ok(true)
    }

    pub(crate) async fn rollback(&mut self, depth: Option<u64>) -> Result<()> {
        node_info!("anvil_rollback");
        let res = self
            .snapshot_manager
            .rollback(depth)
            .map_err(|err| Error::Backend(BackendError::Client(err)))?;

        self.on_revert_update(res).await?;

        Ok(())
    }

    async fn get_block_transaction_count_by_hash(&self, block_hash: B256) -> Result<Option<U256>> {
        let block_hash = H256::from_slice(block_hash.as_slice());
        Ok(self.eth_rpc_client.receipts_count_per_block(&block_hash).await.map(U256::from))
    }

    async fn get_block_transaction_count_by_number(
        &self,
        block_number: BlockNumberOrTag,
    ) -> Result<Option<U256>> {
        let Some(block) = self.get_block_by_number(block_number, false).await? else {
            return Ok(None);
        };
        Ok(self.eth_rpc_client.receipts_count_per_block(&block.hash).await.map(U256::from))
    }

    async fn get_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        transaction_index: U256,
    ) -> Result<Option<TransactionInfo>> {
        let Some(receipt) = self
            .eth_rpc_client
            .receipt_by_hash_and_index(
                &H256::from_slice(block_hash.as_ref()),
                transaction_index.try_into().map_err(|_| EthRpcError::ConversionError)?,
            )
            .await
        else {
            return Ok(None);
        };

        let Some(signed_tx) =
            self.eth_rpc_client.signed_tx_by_hash(&receipt.transaction_hash).await
        else {
            return Ok(None);
        };

        Ok(Some(TransactionInfo::new(&receipt, signed_tx)))
    }

    async fn get_transaction_by_block_number_and_index(
        &self,
        block: BlockNumberOrTag,
        transaction_index: U256,
    ) -> Result<Option<TransactionInfo>> {
        let Some(block) = self
            .eth_rpc_client
            .block_by_number_or_tag(&ReviveBlockNumberOrTag::from(block).inner())
            .await?
        else {
            return Ok(None);
        };
        self.get_transaction_by_block_hash_and_index(
            B256::from_slice(block.hash().as_ref()),
            transaction_index,
        )
        .await
    }

    async fn get_transaction_by_hash(
        &self,
        transaction_hash: B256,
    ) -> Result<Option<TransactionInfo>> {
        let tx_hash = H256::from_slice(transaction_hash.as_ref());
        let receipt = self.eth_rpc_client.receipt(&tx_hash).await;
        let signed_tx = self.eth_rpc_client.signed_tx_by_hash(&tx_hash).await;
        if let (Some(receipt), Some(signed_tx)) = (receipt, signed_tx) {
            return Ok(Some(TransactionInfo::new(&receipt, signed_tx)));
        }

        Ok(None)
    }

    async fn fee_history(
        &self,
        block_count: U256,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> Result<FeeHistoryResult> {
        let block_count: u32 = block_count.try_into().map_err(|_| EthRpcError::ConversionError)?;
        let result = self
            .eth_rpc_client
            .fee_history(
                block_count,
                ReviveBlockNumberOrTag::from(newest_block).inner(),
                reward_percentiles,
            )
            .await?;
        Ok(result)
    }

    async fn max_priority_fee_per_gas(&self) -> Result<sp_core::U256> {
        let gas_price = self.gas_price().await?;
        Ok(Permill::from_percent(20).mul_ceil(gas_price))
    }

    pub fn accounts(&self) -> Result<Vec<H160>> {
        // Spoiler this method will be modified extensively after implementing
        // the wallet related RPCs.
        node_info!("eth_accounts");
        let mut unique = HashSet::new();
        for acc in &self.wallet.accounts {
            unique.insert(acc.address());
        }
        for acc in &self.impersonation_manager.impersonated_accounts {
            unique.insert(*acc);
        }
        Ok(unique.into_iter().collect())
    }

    async fn get_logs(&self, filter: Filter) -> Result<FilterResults> {
        let logs = self.eth_rpc_client.logs(Some(ReviveFilter::from(filter).into_inner())).await?;
        Ok(FilterResults::Logs(logs))
    }

    // State injector RPCs
    fn set_chain_id(&self, chain_id: u64) -> Result<()> {
        node_info!("anvil_setChainId");

        let latest_block = self.latest_block();
        self.backend.inject_chain_id(latest_block, chain_id);

        Ok(())
    }

    fn set_balance(&self, address: Address, value: U256) -> Result<()> {
        node_info!("anvil_setBalance");

        let latest_block = self.latest_block();

        let (new_balance, dust) = self.construct_balance_with_dust(latest_block, value)?;

        let account_id = self.get_account_id(latest_block, address)?;
        self.set_frame_system_balance(latest_block, account_id, new_balance)?;

        let mut revive_account_info = self
            .backend
            .read_revive_account_info(latest_block, address)?
            .unwrap_or(ReviveAccountInfo { account_type: AccountType::EOA, dust: 0 });

        if revive_account_info.dust != dust {
            revive_account_info.dust = dust;

            self.backend.inject_revive_account_info(latest_block, address, revive_account_info);
        }

        Ok(())
    }

    fn set_nonce(&self, address: Address, value: U256) -> Result<()> {
        node_info!("anvil_setNonce");

        let latest_block = self.latest_block();

        let account_id = self.get_account_id(latest_block, address)?;

        let mut account_info = self
            .backend
            .read_system_account_info(latest_block, account_id.clone())?
            .unwrap_or_else(|| SystemAccountInfo { providers: 1, ..Default::default() });

        account_info.nonce = value.try_into().map_err(|_| Error::NonceOverflow)?;

        self.backend.inject_system_account_info(latest_block, account_id, account_info);

        Ok(())
    }

    fn set_storage_at(&self, address: Address, key: U256, value: B256) -> Result<()> {
        node_info!("anvil_setStorageAt");

        let latest_block = self.latest_block();

        let Some(ReviveAccountInfo { account_type: AccountType::Contract(contract_info), .. }) =
            self.backend.read_revive_account_info(latest_block, address)?
        else {
            return Ok(());
        };

        self.backend.inject_child_storage(
            latest_block,
            contract_info.trie_id.to_vec(),
            key.to_be_bytes_vec(),
            value.to_vec(),
        );

        Ok(())
    }

    fn set_code(&self, address: Address, bytes: alloy_primitives::Bytes) -> Result<()> {
        node_info!("anvil_setCode");

        let latest_block = self.latest_block();

        let account_id = self.get_account_id(latest_block, address)?;

        let code_hash = H256(keccak_256(&bytes));

        let maybe_system_account_info =
            self.backend.read_system_account_info(latest_block, account_id.clone())?;
        let nonce = maybe_system_account_info.as_ref().map(|info| info.nonce).unwrap_or_default();

        if maybe_system_account_info.is_none() {
            self.set_frame_system_balance(
                latest_block,
                account_id.clone(),
                substrate_runtime::currency::DOLLARS,
            )?;
        }

        let mut old_code_info = None;
        let revive_account_info = match self
            .backend
            .read_revive_account_info(latest_block, address)?
        {
            None => {
                let contract_info = new_contract_info(&address, code_hash, nonce);

                ReviveAccountInfo { account_type: AccountType::Contract(contract_info), dust: 0 }
            }
            Some(ReviveAccountInfo { account_type: AccountType::EOA, dust }) => {
                let contract_info = new_contract_info(&address, code_hash, nonce);

                ReviveAccountInfo { account_type: AccountType::Contract(contract_info), dust }
            }
            Some(ReviveAccountInfo {
                account_type: AccountType::Contract(mut contract_info),
                dust,
            }) => {
                if let Some(code_info) =
                    self.backend.read_code_info(latest_block, contract_info.code_hash)?
                {
                    if code_info.refcount == 1 && contract_info.code_hash != code_hash {
                        // Remove the pristine code and code info for the old hash.
                        self.backend.inject_pristine_code(
                            latest_block,
                            contract_info.code_hash,
                            None,
                        );
                        self.backend.inject_code_info(latest_block, contract_info.code_hash, None);
                    }

                    old_code_info = Some(code_info);
                }

                contract_info.code_hash = code_hash;

                ReviveAccountInfo { account_type: AccountType::Contract(contract_info), dust }
            }
        };

        self.backend.inject_revive_account_info(latest_block, address, revive_account_info);

        let code_info = old_code_info
            .map(|mut code_info| {
                code_info.code_len = bytes.len() as u32;
                code_info.code_type = ByteCodeType::Evm;
                code_info
            })
            .unwrap_or_else(|| CodeInfo {
                owner: <[u8; 32]>::from(account_id).into(),
                deposit: Default::default(),
                refcount: 1,
                code_len: bytes.len() as u32,
                behaviour_version: 0,
                code_type: ByteCodeType::Evm,
            });

        self.backend.inject_pristine_code(latest_block, code_hash, Some(bytes));
        self.backend.inject_code_info(latest_block, code_hash, Some(code_info));

        Ok(())
    }

    // ----- Helpers
    async fn update_block_provider_on_revert(&self, info: &Info<OpaqueBlock>) -> Result<()> {
        let new_best_block = self.block_provider.block_by_number(info.best_number).await?;
        let new_finalized_block =
            self.block_provider.block_by_number(info.finalized_number).await?;

        if let Some(block) = new_best_block.and_then(Arc::into_inner) {
            self.block_provider.update_latest(block, SubscriptionType::BestBlocks).await;
        }

        if let Some(block) = new_finalized_block.and_then(Arc::into_inner) {
            self.block_provider.update_latest(block, SubscriptionType::FinalizedBlocks).await;
        }

        Ok(())
    }

    async fn update_time_on_revert(&self, best_hash: Hash) -> Result<()> {
        let timestamp = self.backend.read_timestamp(best_hash)?;
        self.mining_engine.set_time(Duration::from_millis(timestamp));
        Ok(())
    }

    async fn on_revert_update(&self, revert_info: RevertInfo) -> Result<()> {
        if revert_info.reverted > 0 {
            self.update_block_provider_on_revert(&revert_info.info).await?;
        }

        let hash = self
            .get_block_hash_for_tag(Some(BlockId::Number(BlockNumberOrTag::Number(
                revert_info.info.best_number.into(),
            ))))
            .await?;
        self.update_time_on_revert(hash).await?;

        Ok(())
    }

    async fn get_block_hash_for_tag(&self, block_id: Option<BlockId>) -> Result<H256> {
        self.eth_rpc_client
            .block_hash_for_tag(ReviveBlockId::from(block_id).inner())
            .await
            .map_err(Error::from)
    }

    fn get_account_id(&self, block: Hash, address: Address) -> Result<AccountId> {
        Ok(self.client.runtime_api().account_id(block, ReviveAddress::from(address).inner())?)
    }

    fn construct_balance_with_dust(&self, block: Hash, value: U256) -> Result<(Balance, u32)> {
        self.client
            .runtime_api()
            .new_balance_with_dust(block, SubstrateU256::from(value).inner())?
            .map_err(|_| Error::BalanceConversion)
    }

    fn latest_block(&self) -> H256 {
        self.backend.blockchain().info().best_hash
    }

    fn set_frame_system_balance(
        &self,
        latest_block: H256,
        account_id: AccountId,
        balance: Balance,
    ) -> Result<()> {
        let mut total_issuance = self.backend.read_total_issuance(latest_block)?;

        let mut system_account_info = self
            .backend
            .read_system_account_info(latest_block, account_id.clone())?
            .unwrap_or_else(|| SystemAccountInfo { providers: 1, ..Default::default() });

        if let Some(diff) = balance.checked_sub(system_account_info.data.free) {
            total_issuance = total_issuance.saturating_add(diff);
        } else {
            total_issuance = total_issuance.saturating_sub(system_account_info.data.free - balance);
        }

        system_account_info.data.free = balance;

        self.backend.inject_system_account_info(latest_block, account_id, system_account_info);
        self.backend.inject_total_issuance(latest_block, total_issuance);

        Ok(())
    }
}

fn new_contract_info(address: &Address, code_hash: H256, nonce: Nonce) -> ContractInfo {
    let address = H160::from_slice(address.as_slice());

    let trie_id = {
        let buf = ("bcontract_trie_v1", address, nonce).using_encoded(BlakeTwo256::hash);
        buf.as_ref()
            .to_vec()
            .try_into()
            .expect("Runtime uses a reasonable hash size. Hence sizeof(T::Hash) <= 128; qed")
    };

    ContractInfo {
        trie_id,
        code_hash,
        storage_bytes: 0,
        storage_items: 0,
        storage_byte_deposit: 0,
        storage_item_deposit: 0,
        storage_base_deposit: 0,
        immutable_data_len: 0,
    }
}

async fn create_online_client(
    substrate_service: &Service,
    rpc_client: RpcClient,
) -> Result<OnlineClient<SrcChainConfig>> {
    let genesis_block_number = substrate_service.genesis_block_number.try_into().map_err(|_| {
        Error::InternalError(format!(
            "Genesis block number {} is too large for u32 (max: {})",
            substrate_service.genesis_block_number,
            u32::MAX
        ))
    })?;

    let Some(genesis_hash) = substrate_service.client.hash(genesis_block_number).ok().flatten()
    else {
        return Err(Error::InternalError(format!(
            "Genesis hash not found for genesis block number {}",
            substrate_service.genesis_block_number
        )));
    };

    let Ok(runtime_version) = substrate_service.client.runtime_version_at(genesis_hash) else {
        return Err(Error::InternalError(
            "Runtime version not found for given genesis hash".to_string(),
        ));
    };

    let subxt_runtime_version = SubxtRuntimeVersion {
        spec_version: runtime_version.spec_version,
        transaction_version: runtime_version.transaction_version,
    };

    let Ok(supported_metadata_versions) =
        substrate_service.client.runtime_api().metadata_versions(genesis_hash)
    else {
        return Err(Error::InternalError("Unable to fetch metadata versions".to_string()));
    };

    let Some(latest_metadata_version) = supported_metadata_versions.into_iter().max() else {
        return Err(Error::InternalError("No stable metadata versions supported".to_string()));
    };

    let opaque_metadata = substrate_service
        .client
        .runtime_api()
        .metadata_at_version(genesis_hash, latest_metadata_version)
        .map_err(|_| {
            Error::InternalError("Failed to get runtime API for genesis hash".to_string())
        })?
        .ok_or_else(|| {
            Error::InternalError(format!(
                "Metadata not found for version {latest_metadata_version} at genesis hash"
            ))
        })?;
    let subxt_metadata = SubxtMetadata::decode(&mut (*opaque_metadata).as_slice())
        .map_err(|_| Error::InternalError("Unable to decode metadata".to_string()))?;

    OnlineClient::<SrcChainConfig>::from_rpc_client_with(
        genesis_hash,
        subxt_runtime_version,
        subxt_metadata,
        rpc_client,
    )
    .map_err(|err| {
        Error::InternalError(format!("Failed to initialize the subxt online client: {err}"))
    })
}

async fn create_revive_rpc_client(
    api: OnlineClient<SrcChainConfig>,
    rpc_client: RpcClient,
    rpc: LegacyRpcMethods<SrcChainConfig>,
    block_provider: SubxtBlockInfoProvider,
    task_spawn_handle: SpawnTaskHandle,
) -> Result<EthRpcClient> {
    let (pool, keep_latest_n_blocks) = {
        // see sqlite in-memory issue: https://github.com/launchbadge/sqlx/issues/2510
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect("sqlite::memory:")
            .await
            .map_err(|err| {
                Error::ReviveRpc(EthRpcError::ClientError(ClientError::SqlxError(err)))
            })?;

        (pool, Some(100))
    };

    let receipt_extractor = ReceiptExtractor::new_with_custom_address_recovery(
        api.clone(),
        None,
        Arc::new(|signed_tx: &TransactionSigned| {
            let sig = signed_tx.raw_signature()?;
            if sig[..12] == [0; 12] && sig[32..64] == [0; 32] {
                let mut res = [0; 20];
                res.copy_from_slice(&sig[12..32]);
                Ok(H160::from(res))
            } else {
                signed_tx.recover_eth_address()
            }
        }),
    )
    .await
    .map_err(|err| Error::ReviveRpc(EthRpcError::ClientError(err)))?;

    let receipt_provider = ReceiptProvider::new(
        pool,
        block_provider.clone(),
        receipt_extractor.clone(),
        keep_latest_n_blocks,
    )
    .await
    .map_err(|err| Error::ReviveRpc(EthRpcError::ClientError(ClientError::SqlxError(err))))?;

    let eth_rpc_client = EthRpcClient::new(api, rpc_client, rpc, block_provider, receipt_provider)
        .await
        .map_err(Error::from)?;
    let eth_rpc_client_clone = eth_rpc_client.clone();
    task_spawn_handle.spawn("block-subscription", "None", async move {
        let eth_rpc_client = eth_rpc_client_clone;
        let best_future =
            eth_rpc_client.subscribe_and_cache_new_blocks(SubscriptionType::BestBlocks);
        let finalized_future =
            eth_rpc_client.subscribe_and_cache_new_blocks(SubscriptionType::FinalizedBlocks);
        let res = tokio::try_join!(best_future, finalized_future).map(|_| ());
        if let Err(err) = res {
            panic!("Block subscription task failed: {err:?}")
        }
    });

    Ok(eth_rpc_client)
}
