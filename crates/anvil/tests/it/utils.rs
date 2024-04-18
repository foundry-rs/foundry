use alloy_json_abi::JsonAbi;
use alloy_network::EthereumSigner;
use alloy_primitives::{Address as aAddress, Bytes};
use ethers::{
    addressbook::contract,
    contract::ContractInstance,
    middleware::Middleware,
    prelude::DeploymentTxFactory,
    types::{Address, Chain},
};
use foundry_common::provider::{
    alloy::{
        get_http_provider, ProviderBuilder as AlloyProviderBuilder,
        RetryProvider as AlloyRetryProvider, RetryProviderWithSigner,
    },
    ethers::{ProviderBuilder, RetryProvider},
};
use std::borrow::Borrow;

use crate::abi::AlloyGreeter;

/// Returns a set of various contract addresses
pub fn contract_addresses(chain: Chain) -> Vec<Address> {
    vec![
        contract("dai").unwrap().address(chain).unwrap(),
        contract("usdc").unwrap().address(chain).unwrap(),
        contract("weth").unwrap().address(chain).unwrap(),
        contract("uniswapV3Factory").unwrap().address(chain).unwrap(),
        contract("uniswapV3SwapRouter02").unwrap().address(chain).unwrap(),
    ]
}

pub fn http_provider(http_endpoint: &str) -> AlloyRetryProvider {
    get_http_provider(http_endpoint)
}

pub fn http_provider_with_signer(
    http_endpoint: &str,
    signer: EthereumSigner,
) -> RetryProviderWithSigner {
    AlloyProviderBuilder::new(http_endpoint)
        .build_with_signer(signer)
        .expect("failed to build Alloy HTTP provider with signer")
}

pub fn ws_provider(ws_endpoint: &str) -> AlloyRetryProvider {
    AlloyProviderBuilder::new(ws_endpoint).build().expect("failed to build Alloy WS provider")
}

pub fn ws_provider_with_signer(
    ws_endpoint: &str,
    signer: EthereumSigner,
) -> RetryProviderWithSigner {
    AlloyProviderBuilder::new(ws_endpoint)
        .build_with_signer(signer)
        .expect("failed to build Alloy WS provider with signer")
}

pub async fn ipc_provider(ipc_endpoint: &str) -> AlloyRetryProvider {
    AlloyProviderBuilder::new(ipc_endpoint).build().expect("failed to build Alloy IPC provider")
}

pub async fn ipc_provider_with_signer(
    ipc_endpoint: &str,
    signer: EthereumSigner,
) -> RetryProviderWithSigner {
    AlloyProviderBuilder::new(ipc_endpoint)
        .build_with_signer(signer)
        .expect("failed to build Alloy IPC provider with signer")
}

/// Builds an ethers HTTP [RetryProvider]
pub fn ethers_http_provider(http_endpoint: &str) -> RetryProvider {
    ProviderBuilder::new(http_endpoint).build().expect("failed to build ethers HTTP provider")
}

/// Builds an ethers ws [RetryProvider]
pub fn ethers_ws_provider(ws_endpoint: &str) -> RetryProvider {
    ProviderBuilder::new(ws_endpoint).build().expect("failed to build ethers HTTP provider")
}

/// Builds an ethers ws [RetryProvider]
pub fn ethers_ipc_provider(ipc_endpoint: Option<String>) -> Option<RetryProvider> {
    ProviderBuilder::new(&ipc_endpoint?).build().ok()
}

/// Temporary helper trait for compatibility with ethers
pub trait ContractInstanceCompat<B, M>
where
    B: Borrow<M>,
    M: Middleware,
{
    fn new_compat(address: Address, abi: JsonAbi, client: B) -> Self;
}

impl<B, M> ContractInstanceCompat<B, M> for ContractInstance<B, M>
where
    B: Borrow<M>,
    M: Middleware,
{
    fn new_compat(address: Address, abi: JsonAbi, client: B) -> Self {
        let json = serde_json::to_string(&abi).unwrap();
        ContractInstance::new(
            address,
            serde_json::from_str::<ethers::abi::Abi>(&json).unwrap(),
            client,
        )
    }
}

pub trait DeploymentTxFactoryCompat<B, M>
where
    B: Borrow<M> + Clone,
    M: Middleware,
{
    fn new_compat(abi: JsonAbi, bytecode: Bytes, client: B) -> Self;
}

impl<B, M> DeploymentTxFactoryCompat<B, M> for DeploymentTxFactory<B, M>
where
    B: Borrow<M> + Clone,
    M: Middleware,
{
    fn new_compat(abi: JsonAbi, bytecode: Bytes, client: B) -> Self {
        let json = serde_json::to_string(&abi).unwrap();
        DeploymentTxFactory::new(
            serde_json::from_str::<ethers::abi::Abi>(&json).unwrap(),
            bytecode.as_ref().to_vec().into(),
            client,
        )
    }
}
