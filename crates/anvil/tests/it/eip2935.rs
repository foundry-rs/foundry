use crate::utils::http_provider;
use alloy_eips::eip2935::HISTORY_STORAGE_ADDRESS;
use alloy_provider::Provider;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_contract_deployed_at_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let code = provider.get_code_at(HISTORY_STORAGE_ADDRESS).await.unwrap();
    assert!(!code.is_empty(), "EIP-2935 history storage contract should be deployed at genesis");
}
