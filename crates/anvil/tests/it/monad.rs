use alloy_consensus::{SidecarBuilder, SimpleCoder};
use alloy_network::{ReceiptResponse, TransactionBuilder, TransactionBuilder4844};
use alloy_primitives::{Address, Bytes, U256, address, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag, TransactionRequest, anvil::Forking};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::MonadHardfork;

const RESERVE_BALANCE_ADDRESS: Address = address!("0x0000000000000000000000000000000000001001");
const DIPPED_INTO_RESERVE_SELECTOR: [u8; 4] = hex!("3a61584e");
const EIP170_CODE_SIZE_LIMIT: usize = 0x6000;
const EIP3860_INITCODE_SIZE_LIMIT: usize = 0xc000;
const EIP7825_TX_GAS_LIMIT_CAP: u64 = 0x1000000;
const MONAD_TX_GAS_LIMIT_CAP: u64 = 30_000_000;

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
    let config = monad_eight_config();
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
async fn monad_can_mine_contract_larger_than_eip3860_initcode_limit() {
    let config = monad_eight_config();
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let init_code = large_contract_init_code(EIP3860_INITCODE_SIZE_LIMIT);
    assert!(init_code.len() > EIP3860_INITCODE_SIZE_LIMIT);

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_input(init_code)
        .with_gas_limit(25_000_000);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
    let contract = receipt.contract_address.expect("deployment should create a contract");
    let code = provider.get_code_at(contract).await.unwrap();
    assert_eq!(code.len(), EIP3860_INITCODE_SIZE_LIMIT);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_allows_tx_gas_limit_above_eip7825_cap() {
    let config = monad_eight_config().enable_tx_gas_limit(true);
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let gas_limit = 20_000_000;
    assert!(gas_limit > EIP7825_TX_GAS_LIMIT_CAP);

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_value(U256::from(1))
        .with_gas_limit(gas_limit);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_rejects_tx_gas_limit_above_monad_cap() {
    let config = monad_eight_config().enable_tx_gas_limit(true).with_gas_limit(Some(40_000_000));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_value(U256::from(1))
        .with_gas_limit(MONAD_TX_GAS_LIMIT_CAP + 1);
    let err = provider.send_transaction(tx.into()).await.unwrap_err().to_string();

    assert!(err.contains("tx.gas_limit > env.cfg.tx_gas_limit_cap"), "unexpected error: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_rejects_eip4844_blob_transactions() {
    let config = monad_nine_config();
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Hello World");
    let sidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar)
        .with_gas_limit(100_000)
        .with_value(U256::from(5));
    let err = provider.send_transaction(WithOtherFields::new(tx)).await.unwrap_err().to_string();

    assert!(
        err.contains("EIP-4844 blob transactions are not supported on Monad"),
        "unexpected error: {err}"
    );
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

#[tokio::test(flavor = "multi_thread")]
async fn monad_safe_and_finalized_block_tags_use_configured_epoch_slots() {
    let slots_in_an_epoch = 3;
    let config = monad_eight_config().with_slots_in_an_epoch(slots_in_an_epoch);
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    api.anvil_mine(Some(U256::from(8)), None).await.unwrap();
    let latest = provider.get_block_number().await.unwrap();
    assert_eq!(latest, 8);

    let safe = provider.get_block(BlockId::Number(BlockNumberOrTag::Safe)).await.unwrap().unwrap();
    assert_eq!(safe.header.number, latest - slots_in_an_epoch);

    let finalized =
        provider.get_block(BlockId::Number(BlockNumberOrTag::Finalized)).await.unwrap().unwrap();
    assert_eq!(finalized.header.number, latest - slots_in_an_epoch * 2);

    let fee_history = api.fee_history(U256::from(1), BlockNumberOrTag::Safe, vec![]).await.unwrap();
    assert_eq!(fee_history.oldest_block, latest - slots_in_an_epoch);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_safe_and_finalized_block_tags_fall_back_to_genesis_before_epoch() {
    let config = monad_eight_config().with_slots_in_an_epoch(3);
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
    let genesis = provider.get_block(BlockId::number(0)).await.unwrap().unwrap();

    let safe = provider.get_block(BlockId::Number(BlockNumberOrTag::Safe)).await.unwrap().unwrap();
    assert_eq!(safe.header.number, genesis.header.number);
    assert_eq!(safe.header.hash, genesis.header.hash);

    let finalized =
        provider.get_block(BlockId::Number(BlockNumberOrTag::Finalized)).await.unwrap().unwrap();
    assert_eq!(finalized.header.number, genesis.header.number);
    assert_eq!(finalized.header.hash, genesis.header.hash);

    let fee_history =
        api.fee_history(U256::from(1), BlockNumberOrTag::Finalized, vec![]).await.unwrap();
    assert_eq!(fee_history.oldest_block, genesis.header.number);
}

fn monad_nine_config() -> NodeConfig {
    NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into()))
}

fn monad_eight_config() -> NodeConfig {
    NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadEight.into()))
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
