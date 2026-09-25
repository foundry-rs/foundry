//! Gas related tests

use crate::utils::http_provider_with_signer;
use alloy_chains::NamedChain;
use alloy_genesis::Genesis;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U64, U256, bytes, uint};
use alloy_provider::Provider;
use alloy_rpc_types::{AccessList, AccessListItem, BlockId, BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{
    EthereumHardfork, NodeConfig,
    eth::{
        error::{BlockchainError, InvalidTransactionError},
        fees::INITIAL_BASE_FEE,
    },
    spawn,
};
use foundry_evm::constants::HARDHAT_CONSOLE_ADDRESS;
use foundry_evm_networks::arbitrum;
use revm::context_interface::block::BlobExcessGasAndPrice;

const GAS_TRANSFER: u64 = 21_000;

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_limit_applied_from_config() {
    let (api, _handle) = spawn(NodeConfig::test().with_gas_limit(Some(10_000_000))).await;

    assert_eq!(api.gas_limit(), uint!(10_000_000_U256));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_limit_disabled_from_config() {
    let (api, _handle) = spawn(NodeConfig::test().disable_block_gas_limit(true)).await;

    // see https://github.com/foundry-rs/foundry/pull/8933
    assert_eq!(api.gas_limit(), U256::from(U64::MAX));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_full_block() {
    let (_api, handle) = spawn(
        NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE)).with_gas_limit(Some(GAS_TRANSFER)),
    )
    .await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    assert!(next_base_fee > base_fee);

    // max increase, full block
    assert_eq!(next_base_fee, INITIAL_BASE_FEE + 125_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_half_block() {
    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_base_fee(Some(INITIAL_BASE_FEE))
            .with_gas_limit(Some(GAS_TRANSFER * 2)),
    )
    .await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let tx = TransactionRequest::default().to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // unchanged, half block
    assert_eq!(next_base_fee, { INITIAL_BASE_FEE });
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basefee_empty_block() {
    let (api, handle) = spawn(NodeConfig::test().with_base_fee(Some(INITIAL_BASE_FEE))).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let tx = TransactionRequest::default().with_to(Address::random()).with_value(U256::from(1337));
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // mine empty block
    api.mine_one().await.unwrap();

    let next_base_fee = provider
        .get_block(BlockId::latest())
        .await
        .unwrap()
        .unwrap()
        .header
        .base_fee_per_gas
        .unwrap();

    // empty block, decreased base fee
    assert!(next_base_fee < base_fee);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_respect_base_fee() {
    let base_fee = 50u128;
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee as u64))).await;

    let provider = handle.http_provider();

    let tx = TransactionRequest::default().with_to(Address::random()).with_value(U256::from(100));
    let mut tx = WithOtherFields::new(tx);

    let mut underpriced = tx.clone();
    underpriced.set_gas_price(base_fee - 1);

    let res = provider.send_transaction(underpriced).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("max fee per gas less than block base fee"));

    tx.set_gas_price(base_fee);
    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tip_above_fee_cap() {
    let base_fee = 50u128;
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee as u64))).await;

    let provider = handle.http_provider();

    let tx = TransactionRequest::default()
        .max_fee_per_gas(base_fee)
        .max_priority_fee_per_gas(base_fee + 1)
        .with_to(Address::random())
        .with_value(U256::from(100));
    let tx = WithOtherFields::new(tx);

    let res = provider.send_transaction(tx.clone()).await;
    assert!(res.is_err());
    assert!(
        res.unwrap_err()
            .to_string()
            .contains("max priority fee per gas higher than max fee per gas")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_zero_block_fee_history_is_empty() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let history = api.fee_history(U256::ZERO, BlockNumberOrTag::Latest, vec![50.0]).await.unwrap();

    assert_eq!(history.oldest_block, 0);
    assert!(history.base_fee_per_gas.is_empty());
    assert!(history.gas_used_ratio.is_empty());
    assert!(history.reward.is_none());
    assert!(history.base_fee_per_blob_gas.is_empty());
    assert!(history.blob_gas_used_ratio.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_can_use_fee_history() {
    let base_fee = 50u128;
    let (_api, handle) = spawn(NodeConfig::test().with_base_fee(Some(base_fee as u64))).await;
    let provider = handle.http_provider();

    for _ in 0..10 {
        let fee_history = provider.get_fee_history(1, Default::default(), &[]).await.unwrap();
        let next_base_fee = *fee_history.base_fee_per_gas.last().unwrap();

        let tx = TransactionRequest::default()
            .with_to(Address::random())
            .with_value(U256::from(100))
            .with_gas_price(next_base_fee);
        let tx = WithOtherFields::new(tx);

        let receipt =
            provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
        assert!(receipt.inner.inner.is_success());

        let fee_history_after = provider.get_fee_history(1, Default::default(), &[]).await.unwrap();
        let latest_fee_history_fee = *fee_history_after.base_fee_per_gas.first().unwrap() as u64;
        let latest_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();

        assert_eq!(latest_block.header.base_fee_per_gas.unwrap(), latest_fee_history_fee);
        assert_eq!(latest_fee_history_fee, next_base_fee as u64);
    }
}

// `base_fee_per_gas` includes one entry for the block after the requested range. For a
// historical range, that entry must come from the historical child rather than the current
// chain head's next-block fee.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_historical_next_block_fee() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
    let provider = handle.http_provider();
    let blob_params = api.backend.blob_params();
    let blob_update_fraction = u64::try_from(blob_params.update_fraction).unwrap();

    for (base_fee, excess_blob_gas) in [(100, 0), (200, 10_000_000), (300, 20_000_000)] {
        api.anvil_set_next_block_base_fee_per_gas(U256::from(base_fee)).await.unwrap();
        api.backend.fees().set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
            excess_blob_gas,
            blob_update_fraction,
        ));
        api.mine_one().await.unwrap();
    }
    api.anvil_set_next_block_base_fee_per_gas(U256::from(400)).await.unwrap();
    api.backend.fees().set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
        30_000_000,
        blob_update_fraction,
    ));

    let history =
        api.fee_history(U256::from(1), BlockNumberOrTag::Number(1), vec![]).await.unwrap();
    let historical = provider.get_block(BlockId::number(1)).await.unwrap().unwrap();
    let historical_child = provider.get_block(BlockId::number(2)).await.unwrap().unwrap();
    let historical_next_fee = historical_child.header.base_fee_per_gas.unwrap() as u128;
    let historical_blob_fee = historical.header.blob_fee().unwrap();
    let historical_next_blob_fee = historical_child.header.blob_fee().unwrap();

    assert_eq!(history.base_fee_per_gas, vec![100, historical_next_fee]);
    assert_ne!(historical_next_fee, api.base_fee().unwrap().unwrap().to::<u128>());
    assert_eq!(history.base_fee_per_blob_gas, vec![historical_blob_fee, historical_next_blob_fee]);
    assert_ne!(historical_next_blob_fee, api.backend.fees().base_fee_per_blob_gas());
}

// Cache entries from the previous chain must not survive `anvil_reset` merely because the new
// chain reuses the same block number.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_ignores_stale_cache_after_reset() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let old_history =
        api.fee_history(U256::from(1), BlockNumberOrTag::Number(0), vec![]).await.unwrap();
    api.anvil_set_next_block_base_fee_per_gas(U256::from(123)).await.unwrap();
    api.anvil_reset(None).await.unwrap();

    let genesis = provider.get_block(BlockId::number(0)).await.unwrap().unwrap();
    let genesis_base_fee = genesis.header.base_fee_per_gas.unwrap() as u128;
    let new_history =
        api.fee_history(U256::from(1), BlockNumberOrTag::Number(0), vec![]).await.unwrap();

    assert_eq!(genesis_base_fee, 123);
    assert_ne!(old_history.base_fee_per_gas[0], genesis_base_fee);
    assert_eq!(new_history.base_fee_per_gas[0], genesis_base_fee);
    assert_eq!(api.base_fee().unwrap(), Some(U256::from(INITIAL_BASE_FEE)));

    api.mine_one().await.unwrap();
    let first = provider.get_block(BlockId::number(1)).await.unwrap().unwrap();
    assert_eq!(first.header.base_fee_per_gas, Some(INITIAL_BASE_FEE));
}

// Zero gas limits must not serialize gasUsedRatio as null.
#[tokio::test(flavor = "multi_thread")]
async fn test_fee_history_zero_gas_limit_does_not_produce_null_ratio() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    assert!(api.evm_set_block_gas_limit(U256::ZERO).unwrap());
    api.mine_one().await.unwrap();

    // Use HTTP to exercise JSON serialization and client deserialization.
    let fee_history = provider
        .get_fee_history(1, BlockNumberOrTag::Latest, &[])
        .await
        .expect("gasUsedRatio must deserialize as a finite f64, not null");

    let ratio = *fee_history.gas_used_ratio.last().unwrap();
    assert_eq!(ratio, 0.0, "a zero-gas-limit block used none of its (zero) capacity");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_memory_reset_restores_explicit_genesis_base_fee() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::default().into()))
            .with_genesis(Some(Genesis { base_fee_per_gas: Some(0), ..Default::default() })),
    )
    .await;
    let provider = handle.http_provider();

    api.anvil_set_next_block_base_fee_per_gas(U256::from(999)).await.unwrap();
    api.anvil_reset(None).await.unwrap();

    let genesis = provider.get_block(BlockId::number(0)).await.unwrap().unwrap();
    assert_eq!(genesis.header.base_fee_per_gas, Some(0));
    assert_eq!(api.base_fee().unwrap(), Some(U256::ZERO));
    api.mine_one().await.unwrap();
    let first = provider.get_block(BlockId::number(1)).await.unwrap().unwrap();
    assert_eq!(first.header.base_fee_per_gas, Some(0));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_empty_data() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let accounts = handle.dev_accounts().collect::<Vec<_>>();
    let from = accounts[0];
    let to = accounts[1];

    let tx_without_data =
        TransactionRequest::default().with_from(from).with_to(to).with_value(U256::from(1));

    let gas_without_data = api
        .estimate_gas(WithOtherFields::new(tx_without_data), None, Default::default())
        .await
        .unwrap();

    let tx_with_empty_data = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::from(1))
        .with_input(vec![]);

    let gas_with_empty_data = api
        .estimate_gas(WithOtherFields::new(tx_with_empty_data), None, Default::default())
        .await
        .unwrap();

    let tx_with_data = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::from(1))
        .with_input(vec![0x12, 0x34]);

    let gas_with_data = api
        .estimate_gas(WithOtherFields::new(tx_with_data), None, Default::default())
        .await
        .unwrap();

    assert_eq!(gas_without_data, U256::from(GAS_TRANSFER));
    assert_eq!(gas_with_empty_data, U256::from(GAS_TRANSFER));
    assert!(gas_with_data > U256::from(GAS_TRANSFER));
    assert_eq!(gas_without_data, gas_with_empty_data);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_empty_precompile_data() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    let identity_precompile = Address::with_last_byte(4);
    let tx = TransactionRequest::default()
        .with_to(identity_precompile)
        .with_gas_price(1_000_000_000)
        .with_input(vec![]);

    let gas = api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap();

    assert_eq!(gas, U256::from(GAS_TRANSFER + 15));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_block_precompile() {
    let (api, handle) =
        spawn(NodeConfig::test().with_chain_id(Some(NamedChain::Arbitrum as u64))).await;
    let from = handle.dev_accounts().next().unwrap();
    let empty_input = TransactionRequest::default()
        .with_from(from)
        .with_to(arbitrum::ARB_SYS_ADDRESS)
        .with_input(vec![]);

    let err = api
        .estimate_gas(WithOtherFields::new(empty_input), None, Default::default())
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "EVM error PrecompileError");

    let valid_input = TransactionRequest::default()
        .with_from(from)
        .with_to(arbitrum::ARB_SYS_ADDRESS)
        .with_input(Bytes::copy_from_slice(&arbitrum::ARB_BLOCK_NUMBER_SELECTOR));
    let gas = api
        .estimate_gas(WithOtherFields::new(valid_input), None, Default::default())
        .await
        .unwrap();

    assert_eq!(gas, U256::from(0x52a8));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_simple_transfer_checks_funds() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();
    let from = Address::random();

    let tx = TransactionRequest::default().with_from(from).with_to(to).with_value(U256::from(1));
    let err =
        api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap_err();

    assert!(err.to_string().contains("Insufficient funds for gas * price + value"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_simple_transfer_without_from_uses_transfer_fast_path() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();

    let tx = TransactionRequest::default().with_to(to).with_value(U256::from(1));
    let gas = api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap();

    assert_eq!(gas, U256::from(GAS_TRANSFER));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_without_from_with_gas_price_uses_transfer_fast_path() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();

    let tx = TransactionRequest::default().with_to(to).with_gas_price(INITIAL_BASE_FEE as u128);
    let gas = api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap();

    assert_eq!(gas, U256::from(GAS_TRANSFER));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_fee_token_does_not_skip_funds_check_outside_tempo() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();
    let from = Address::random();

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default().with_from(from).with_to(to).with_value(U256::from(1)),
        other: [(
            "feeToken".to_string(),
            serde_json::json!("0x20c0000000000000000000000000000000000001"),
        )]
        .into_iter()
        .collect(),
    };
    let err = api.estimate_gas(tx, None, Default::default()).await.unwrap_err();

    assert!(err.to_string().contains("Insufficient funds for gas * price + value"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimation_with_print_traces() {
    let contract = Address::random();
    let mut results = Vec::new();
    for print_traces in [false, true] {
        let (api, handle) = spawn(NodeConfig::test().with_print_traces(print_traces)).await;
        let provider = handle.http_provider();
        let from = handle.dev_accounts().next().unwrap();
        // Return the old slot value and store calldata[0], reverting after the write if zero.
        api.anvil_set_code(
            contract,
            bytes!("600054600052600035806000551560165760206000f35b60006000fd"),
        )
        .await
        .unwrap();
        api.anvil_set_storage_at(contract, U256::ZERO, B256::with_last_byte(3)).await.unwrap();
        let mut request = WithOtherFields::new(
            TransactionRequest::default()
                .from(from)
                .to(contract)
                .gas_limit(200_000)
                .input(Bytes::copy_from_slice(&U256::from(5).to_be_bytes::<32>()).into()),
        );
        let estimate = api.estimate_gas(request.clone(), None, Default::default()).await.unwrap();
        let access_list = api.create_access_list(request.clone(), None, None).await.unwrap();
        assert_eq!(access_list.error, None);
        let expected_access_list = AccessList::from(vec![AccessListItem {
            address: contract,
            storage_keys: vec![B256::ZERO],
        }]);
        assert_eq!(access_list.access_list, expected_access_list);

        // The estimated limit must execute successfully, while one gas less must fail.
        request.gas = Some(estimate.to());
        let output = api.call(request.clone(), None, Default::default()).await.unwrap();
        assert_eq!(output.as_ref(), U256::from(3).to_be_bytes::<32>());
        request.gas = Some(estimate.to::<u64>() - 1);
        assert!(api.call(request.clone(), None, Default::default()).await.is_err());

        request.gas = Some(200_000);
        request.input = Bytes::from(vec![0; 32]).into();
        let error = api.estimate_gas(request.clone(), None, Default::default()).await.unwrap_err();
        assert!(matches!(
            error,
            BlockchainError::InvalidTransaction(InvalidTransactionError::Revert(Some(data)))
                if data.is_empty()
        ));
        let reverted = api.create_access_list(request, None, None).await.unwrap();
        assert_eq!(reverted.error.as_deref(), Some("execution reverted"));
        assert_eq!(reverted.access_list, expected_access_list);

        // The console collector must remain active: malformed console calldata must revert,
        // rather than succeed as an ordinary call to an address without code.
        let console_request = WithOtherFields::new(
            TransactionRequest::default()
                .from(from)
                .to(HARDHAT_CONSOLE_ADDRESS)
                .gas_limit(200_000)
                .input(bytes!("01").into()),
        );
        let error = api.estimate_gas(console_request, None, Default::default()).await.unwrap_err();
        let BlockchainError::InvalidTransaction(InvalidTransactionError::Revert(Some(data))) =
            error
        else {
            panic!("expected console decoding revert, got {error:?}");
        };
        assert!(!data.is_empty());
        assert_eq!(provider.get_storage_at(contract, U256::ZERO).await.unwrap(), U256::from(3));
        assert_eq!(provider.get_transaction_count(from).await.unwrap(), 0);
        results.push((estimate, access_list, reverted, data));
    }
    assert_eq!(results[0], results[1]);
}
