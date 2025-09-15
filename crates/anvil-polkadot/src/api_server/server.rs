use crate::{
    api_server::ApiRequest,
    logging::LoggingManager,
    macros::node_info,
    substrate_node::{error::ToRpcResponseResult, mining_engine::MiningEngine, service::Service},
};
use alloy_primitives::U256;
use anvil_core::eth::EthRequest;
use anvil_rpc::{error::RpcError, response::ResponseResult};
use futures::{channel::mpsc, StreamExt};
use std::{sync::Arc, time::Duration};

pub struct ApiServer {
    req_receiver: mpsc::Receiver<ApiRequest>,
    logging_manager: LoggingManager,
    mining_engine: Arc<MiningEngine>,
}

impl ApiServer {
    pub fn new(
        substrate_service: &Service,
        req_receiver: mpsc::Receiver<ApiRequest>,
        logging_manager: LoggingManager,
    ) -> Self {
        Self {
            req_receiver,
            logging_manager,
            mining_engine: substrate_service.mining_engine.clone(),
        }
    }

    pub async fn run(mut self) {
        while let Some(msg) = self.req_receiver.next().await {
            let resp = self.execute(msg.req).await;

            msg.resp_sender.send(resp).expect("Dropped receiver");
        }
    }

    pub async fn execute(&mut self, req: EthRequest) -> ResponseResult {
        match req {
            EthRequest::Mine(blocks, interval) => {
                if blocks.is_some_and(|b| u64::try_from(b).is_err()) {
                    return ResponseResult::Error(RpcError::invalid_params(
                        "The number of blocks is too large",
                    ));
                }
                if interval.is_some_and(|i| u64::try_from(i).is_err()) {
                    return ResponseResult::Error(RpcError::invalid_params(
                        "The interval between blocks is too large",
                    ));
                }
                self.mining_engine
                    .mine(blocks.map(|b| b.to()), interval.map(|i| Duration::from_secs(i.to())))
                    .await
                    .to_rpc_result()
            }
            EthRequest::SetIntervalMining(interval) => self
                .mining_engine
                .set_interval_mining(Duration::from_secs(interval))
                .to_rpc_result(),
            EthRequest::GetIntervalMining(()) => {
                self.mining_engine.get_interval_mining().to_rpc_result()
            }
            EthRequest::GetAutoMine(()) => self.mining_engine.get_auto_mine().to_rpc_result(),
            EthRequest::SetAutomine(enabled) => {
                self.mining_engine.set_auto_mine(enabled).to_rpc_result()
            }
            EthRequest::EvmMine(mine) => {
                self.mining_engine.evm_mine(mine.and_then(|p| p.params)).await.to_rpc_result()
            }
            EthRequest::EvmMineDetailed(_mine) => ResponseResult::Error(RpcError::internal_error()),
            //------- TimeMachine---------
            EthRequest::EvmSetBlockTimeStampInterval(time) => self
                .mining_engine
                .set_block_timestamp_interval(Duration::from_secs(time))
                .to_rpc_result(),
            EthRequest::EvmRemoveBlockTimeStampInterval(()) => {
                self.mining_engine.remove_block_timestamp_interval().to_rpc_result()
            }
            EthRequest::EvmSetNextBlockTimeStamp(time) => {
                if time >= U256::from(u64::MAX) {
                    return ResponseResult::Error(RpcError::invalid_params(
                        "The timestamp is too big",
                    ))
                }
                let time = time.to::<u64>();
                self.mining_engine
                    .set_next_block_timestamp(Duration::from_secs(time))
                    .to_rpc_result()
            }
            EthRequest::EvmIncreaseTime(time) => self
                .mining_engine
                .increase_time(Duration::from_secs(time.try_into().unwrap_or(0)))
                .to_rpc_result(),
            EthRequest::EvmSetTime(timestamp) => {
                if timestamp >= U256::from(u64::MAX) {
                    return ResponseResult::Error(RpcError::invalid_params(
                        "The timestamp is too big",
                    ))
                }
                // Make sure here we are not traveling back in time.
                let time = timestamp.to::<u64>();
                self.mining_engine.set_time(Duration::from_secs(time)).to_rpc_result()
            }
            EthRequest::SetLogging(enabled) => {
                node_info!("anvil_setLoggingEnabled");
                self.logging_manager.set_enabled(enabled);
                ResponseResult::Success(serde_json::Value::Bool(true))
            }
            _ => ResponseResult::Error(RpcError::internal_error()),
        }
    }
}
