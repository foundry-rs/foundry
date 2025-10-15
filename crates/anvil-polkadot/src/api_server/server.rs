use crate::{
    api_server::{
        ApiRequest,
        error::{Error, Result, ToRpcResponseResult},
        revive_conversions::{
            AlloyU256, ReviveAddress, ReviveBlockId, SubstrateU256, convert_to_generic_transaction,
        },
    },
    logging::LoggingManager,
    macros::node_info,
    substrate_node::{
        impersonation::ImpersonationManager,
        in_mem_rpc::InMemoryRpcClient,
        mining_engine::MiningEngine,
        service::{
            BackendWithOverlay, Client, Service,
            storage::{
                AccountType, ByteCodeType, CodeInfo, ContractInfo, ReviveAccountInfo,
                SystemAccountInfo,
            },
        },
    },
};
use alloy_eips::BlockId;
use alloy_primitives::{Address, B256, U64, U256};
use alloy_rpc_types::{anvil::MineOptions, request::TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil_core::eth::{EthRequest, Params as MineParams};
use anvil_rpc::response::ResponseResult;
use codec::{Decode, Encode};
use futures::{StreamExt, channel::mpsc};
use polkadot_sdk::{
    pallet_revive::{
        ReviveApi,
        evm::{Account, Block, Bytes, ReceiptInfo, TransactionSigned},
    },
    pallet_revive_eth_rpc::{
        EthRpcError, ReceiptExtractor, ReceiptProvider, SubxtBlockInfoProvider,
        client::{Client as EthRpcClient, ClientError, SubscriptionType},
        subxt_client::{self, SrcChainConfig},
    },
    parachains_common::{AccountId, Hash, Nonce},
    sc_client_api::HeaderBackend,
    sp_api::{Metadata, ProvideRuntimeApi},
    sp_core::{self, H160, H256, Hasher, keccak_256},
    sp_runtime::traits::BlakeTwo256,
};
use sqlx::sqlite::SqlitePoolOptions;
use std::{sync::Arc, time::Duration};
use substrate_runtime::Balance;
use subxt::{
    Metadata as SubxtMetadata, OnlineClient, backend::rpc::RpcClient,
    client::RuntimeVersion as SubxtRuntimeVersion, ext::subxt_rpcs::LegacyRpcMethods,
};

pub struct Wallet {
    accounts: Vec<Account>,
}

pub struct ApiServer {
    req_receiver: mpsc::Receiver<ApiRequest>,
    backend: BackendWithOverlay,
    logging_manager: LoggingManager,
    client: Arc<Client>,
    mining_engine: Arc<MiningEngine>,
    eth_rpc_client: EthRpcClient,
    wallet: Wallet,
    impersonation_manager: ImpersonationManager,
}

impl ApiServer {
    pub async fn new(
        substrate_service: Service,
        req_receiver: mpsc::Receiver<ApiRequest>,
        logging_manager: LoggingManager,
        impersonation_manager: ImpersonationManager,
    ) -> Result<Self> {
        let eth_rpc_client = create_revive_rpc_client(&substrate_service).await?;

        Ok(Self {
            req_receiver,
            logging_manager,
            backend: BackendWithOverlay::new(
                substrate_service.backend.clone(),
                substrate_service.storage_overrides.clone(),
            ),
            client: substrate_service.client.clone(),
            mining_engine: substrate_service.mining_engine.clone(),
            eth_rpc_client,
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
            //------- TimeMachine---------
            EthRequest::EvmMine(mine) => self.evm_mine(mine).await.to_rpc_result(),
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
                self.send_transaction(*request.clone()).await.to_rpc_result()
            }
            EthRequest::EthGetTransactionCount(addr, block) => self
                .get_transaction_count(ReviveAddress::from(addr).inner(), block)
                .await
                .map(|val| AlloyU256::from(val).inner())
                .to_rpc_result(),

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

        let payload = if self.impersonation_manager.is_impersonated(from) {
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

    // ---- Impersonation RPCs
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

    // ----- Helpers
    async fn get_block_hash_for_tag(&self, block_id: Option<BlockId>) -> Result<H256> {
        self.eth_rpc_client
            .block_hash_for_tag(ReviveBlockId::from(block_id).inner())
            .await
            .map_err(Error::from)
    }

    fn chain_id(&self, at: Hash) -> u64 {
        self.backend.read_chain_id(at).expect("Chain ID is populated on genesis")
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

async fn create_revive_rpc_client(substrate_service: &Service) -> Result<EthRpcClient> {
    let rpc_client = RpcClient::new(InMemoryRpcClient(substrate_service.rpc_handlers.clone()));

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

    let api = OnlineClient::<SrcChainConfig>::from_rpc_client_with(
        genesis_hash,
        subxt_runtime_version,
        subxt_metadata,
        rpc_client.clone(),
    )?;
    let rpc = LegacyRpcMethods::<SrcChainConfig>::new(rpc_client.clone());

    let block_provider = SubxtBlockInfoProvider::new(api.clone(), rpc.clone()).await?;

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
    substrate_service.spawn_handle.spawn("block-subscription", "None", async move {
        let eth_rpc_client = eth_rpc_client_clone;
        let best_future =
            eth_rpc_client.subscribe_and_cache_new_blocks(SubscriptionType::BestBlocks);
        let finalized_future =
            eth_rpc_client.subscribe_and_cache_new_blocks(SubscriptionType::FinalizedBlocks);
        let res = tokio::try_join!(best_future, finalized_future).map(|_| ());
        if let Err(err) = res {
            panic!("Block subscription task failed: {err:?}",)
        }
    });
    Ok(eth_rpc_client)
}
