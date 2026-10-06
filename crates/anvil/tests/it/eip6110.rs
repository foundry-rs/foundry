use alloy_eips::{BlockNumberOrTag, eip6110::MAINNET_DEPOSIT_CONTRACT_ADDRESS};
use alloy_network::TransactionBuilder;
use alloy_primitives::bytes;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;

#[tokio::test(flavor = "multi_thread")]
async fn eip6110_pending_block_returns_malformed_deposit_error() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = handle.http_provider();

    // PUSH32 DepositEvent topic, PUSH1 size=0, PUSH1 offset=0, LOG1, STOP.
    api.anvil_set_code(
        MAINNET_DEPOSIT_CONTRACT_ADDRESS,
        bytes!("7f649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c560006000a100"),
    )
    .await
    .unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();
    let from = handle.dev_wallets().next().unwrap().address();
    let _pending = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(from)
                .to(MAINNET_DEPOSIT_CONTRACT_ADDRESS)
                .with_gas_limit(100_000),
        ))
        .await
        .unwrap();

    let error = provider.get_block_by_number(BlockNumberOrTag::Pending).await.unwrap_err();
    let response = error.as_error_resp().expect("should return a JSON-RPC error");
    assert_eq!(response.code, -32603);
    assert_eq!(provider.get_block_number().await.unwrap(), 0);
}
