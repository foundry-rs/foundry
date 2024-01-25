use ethers::{
    addressbook::contract,
    types::{Address, Chain},
};
use foundry_common::provider::ethers::{ProviderBuilder, RetryProvider};

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
