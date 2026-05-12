use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, Bytes, address, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionRequest, anvil::Forking};
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::MonadHardfork;

const RESERVE_BALANCE_ADDRESS: Address = address!("0x0000000000000000000000000000000000001001");
const DIPPED_INTO_RESERVE_SELECTOR: [u8; 4] = hex!("3a61584e");
const EIP170_CODE_SIZE_LIMIT: usize = 0x6000;

#[tokio::test(flavor = "multi_thread")]
async fn monad_nine_exposes_reserve_balance_precompile_for_calls() {
    let config = NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = provider.call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_can_mine_contract_larger_than_eip170_limit() {
    let config = NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadEight.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let runtime_len = EIP170_CODE_SIZE_LIMIT + 1;

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_input(large_contract_init_code(runtime_len))
        .with_gas_limit(10_000_000);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
    let contract = receipt.contract_address.expect("deployment should create a contract");
    let code = provider.get_code_at(contract).await.unwrap();
    assert_eq!(code.len(), runtime_len);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_uses_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await;

    let config = monad_nine_config().with_eth_rpc_url(Some(origin_handle.http_endpoint()));
    let (fork_api, fork_handle) = spawn(config).await;

    let node_info = fork_api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert!(node_info.fork_config.fork_block_number.is_some());

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = fork_handle.http_provider().call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_reset_without_url_preserves_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await;

    let config = monad_nine_config().with_eth_rpc_url(Some(origin_handle.http_endpoint()));
    let (fork_api, fork_handle) = spawn(config).await;

    fork_api
        .anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(0) }))
        .await
        .unwrap();

    let node_info = fork_api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert_eq!(node_info.fork_config.fork_block_number, Some(0));

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = fork_handle.http_provider().call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reset_can_start_forking_with_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await;

    let (api, handle) = spawn(monad_nine_config()).await;

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(origin_handle.http_endpoint()),
        block_number: Some(0),
    }))
    .await
    .unwrap();

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert_eq!(node_info.fork_config.fork_block_number, Some(0));

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = handle.http_provider().call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

fn monad_nine_config() -> NodeConfig {
    NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into()))
}

fn large_contract_init_code(runtime_len: usize) -> Bytes {
    assert!(runtime_len <= u16::MAX as usize);
    const HEADER_LEN: usize = 15;
    let runtime_len = runtime_len as u16;

    let mut code = Vec::with_capacity(HEADER_LEN + runtime_len as usize);
    code.extend_from_slice(&[0x61, (runtime_len >> 8) as u8, runtime_len as u8]);
    code.extend_from_slice(&[0x61, 0x00, HEADER_LEN as u8]);
    code.extend_from_slice(&[0x60, 0x00, 0x39]);
    code.extend_from_slice(&[0x61, (runtime_len >> 8) as u8, runtime_len as u8]);
    code.extend_from_slice(&[0x60, 0x00, 0xf3]);
    code.resize(HEADER_LEN + runtime_len as usize, 0);
    Bytes::from(code)
}
