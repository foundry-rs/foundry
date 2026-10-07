//! Gas related tests

use crate::utils::http_provider_with_signer;
use alloy_chains::NamedChain;
use alloy_genesis::Genesis;
use alloy_network::{EthereumWallet, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U64, U256, bytes, uint};
use alloy_provider::Provider;
use alloy_rpc_types::{
    AccessList, AccessListItem, BlockId, BlockNumberOrTag, TransactionRequest,
    trace::parity::TraceType,
};
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
async fn cap_only_calls_pay_the_base_fee() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    // Returns GASPRICE.
    let contract = Address::repeat_byte(0x3a);
    api.anvil_set_code(contract, bytes!("3a5f5260205ff3")).await.unwrap();
    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let base_fee = block.header.base_fee_per_gas.unwrap() as u128;

    let cap_only =
        TransactionRequest::default().from(from).to(contract).max_fee_per_gas(base_fee * 10);
    let tipped = cap_only.clone().max_priority_fee_per_gas(1);
    for (request, expected) in [(cap_only, base_fee), (tipped, base_fee + 1)] {
        let request = WithOtherFields::new(request);
        let output = provider.call(request.clone()).block(BlockId::latest()).await.unwrap();
        assert_eq!(U256::from_be_slice(&output), U256::from(expected));

        let traced = api
            .trace_call(request, [TraceType::Trace].into_iter().collect(), Some(BlockId::latest()))
            .await
            .unwrap();
        assert_eq!(U256::from_be_slice(&traced.output), U256::from(expected));
    }
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
async fn priced_calls_without_gas_limit_are_capped_by_allowance() {
    for config in [NodeConfig::test(), NodeConfig::test().with_optimism()] {
        priced_calls_without_gas_limit_are_capped_by_allowance_on(config).await;
    }
}

async fn priced_calls_without_gas_limit_are_capped_by_allowance_on(config: NodeConfig) {
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    // The sender can pay for 100_000 gas, far below the block gas limit.
    let from = Address::repeat_byte(0x11);
    let gas_price = 10_000_000_000_000u128;
    api.anvil_set_balance(from, U256::from(gas_price * 100_000)).await.unwrap();

    // Returns GAS, the gas left after the transaction's 21_000 and the opcode's 2.
    let contract = Address::repeat_byte(0x5a);
    api.anvil_set_code(contract, bytes!("5a5f5260205ff3")).await.unwrap();
    let request = TransactionRequest::default().from(from).to(contract).gas_price(gas_price);

    // Sending half of the balance as value halves the gas the sender can pay for.
    let half = request.clone().value(U256::from(gas_price * 50_000));
    for (request, allowance) in [(request.clone(), 100_000), (half, 50_000)] {
        let request = WithOtherFields::new(request);
        let output = provider.call(request.clone()).await.unwrap();
        assert_eq!(U256::from_be_slice(&output), U256::from(allowance - 21_002));

        let traced =
            api.trace_call(request, [TraceType::Trace].into_iter().collect(), None).await.unwrap();
        assert_eq!(traced.output, output);
    }

    // An explicit gas limit is not lowered to what the sender can pay for, and a sender that
    // cannot pay for any transaction still fails the funds check.
    let unfunded = request.clone().from(Address::repeat_byte(0x12));
    for request in [request.gas_limit(200_000), unfunded] {
        let error = provider.call(WithOtherFields::new(request)).await.unwrap_err();
        let error = error.as_error_resp().unwrap();
        assert_eq!(
            (error.code, error.message.as_ref()),
            (-32003, "Insufficient funds for gas * price + value")
        );
    }
}

#[cfg(feature = "optimism")]
#[tokio::test(flavor = "multi_thread")]
async fn priced_deposit_calls_without_gas_limit_keep_default_budget() {
    let (api, handle) = spawn(NodeConfig::test().with_optimism()).await;
    let provider = handle.http_provider();
    let from = Address::repeat_byte(0x11);
    let gas_price = 10_000_000_000_000u128;
    api.anvil_set_balance(from, U256::from(gas_price * 100_000)).await.unwrap();

    let contract = Address::repeat_byte(0x5a);
    api.anvil_set_code(contract, bytes!("5a5f5260205ff3")).await.unwrap();
    let gas_limit = api.gas_limit();
    // Deposits mint funds before paying for execution, so the pre-mint balance must not
    // lower their default gas budget.
    let request = WithOtherFields {
        inner: TransactionRequest::default()
            .from(from)
            .to(contract)
            .gas_price(gas_price)
            .transaction_type(0x7e),
        other: serde_json::json!({
            "sourceHash": B256::repeat_byte(0x01),
            "mint": gas_limit * U256::from(gas_price),
            "isSystemTx": false,
        })
        .try_into()
        .unwrap(),
    };
    let output = provider.call(request.clone()).await.unwrap();
    assert_eq!(U256::from_be_slice(&output), gas_limit - U256::from(21_002));
    let traced =
        api.trace_call(request, [TraceType::Trace].into_iter().collect(), None).await.unwrap();
    assert_eq!(traced.output, output);
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

    let history = api.fee_history(U256::ONE, BlockNumberOrTag::Number(1), vec![]).await.unwrap();
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
        api.fee_history(U256::ONE, BlockNumberOrTag::Number(0), vec![]).await.unwrap();
    api.anvil_set_next_block_base_fee_per_gas(U256::from(123)).await.unwrap();
    api.anvil_reset(None).await.unwrap();

    let genesis = provider.get_block(BlockId::number(0)).await.unwrap().unwrap();
    let genesis_base_fee = genesis.header.base_fee_per_gas.unwrap() as u128;
    let new_history =
        api.fee_history(U256::ONE, BlockNumberOrTag::Number(0), vec![]).await.unwrap();

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
        TransactionRequest::default().with_from(from).with_to(to).with_value(U256::ONE);

    let gas_without_data = api
        .estimate_gas(WithOtherFields::new(tx_without_data), None, Default::default())
        .await
        .unwrap();

    let tx_with_empty_data = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::ONE)
        .with_input(vec![]);

    let gas_with_empty_data = api
        .estimate_gas(WithOtherFields::new(tx_with_empty_data), None, Default::default())
        .await
        .unwrap();

    let tx_with_data = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(U256::ONE)
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
        .with_input(Bytes::from(arbitrum::ARB_BLOCK_NUMBER_SELECTOR));
    let gas = api
        .estimate_gas(WithOtherFields::new(valid_input), None, Default::default())
        .await
        .unwrap();

    // 21000 base, 64 for the four calldata bytes and 803 inside ArbSys.
    assert_eq!(gas, U256::from(21_867));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_simple_transfer_checks_funds() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();
    let from = Address::random();

    let tx = TransactionRequest::default().with_from(from).with_to(to).with_value(U256::ONE);
    let err =
        api.estimate_gas(WithOtherFields::new(tx), None, Default::default()).await.unwrap_err();

    assert!(err.to_string().contains("Insufficient funds for gas * price + value"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_simple_transfer_without_from_uses_transfer_fast_path() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let to = handle.dev_accounts().next().unwrap();

    let tx = TransactionRequest::default().with_to(to).with_value(U256::ONE);
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
        inner: TransactionRequest::default().with_from(from).with_to(to).with_value(U256::ONE),
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
                .input(Bytes::copy_from_slice(B256::with_last_byte(5).as_slice()).into()),
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
        assert_eq!(output.as_ref(), B256::with_last_byte(3).0);
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

#[tokio::test(flavor = "multi_thread")]
async fn zero_fee_calls_observe_zero_base_fee() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    // Returns BASEFEE.
    let contract = Address::repeat_byte(0x48);
    api.anvil_set_code(contract, bytes!("485f5260205ff3")).await.unwrap();
    let base_fee = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let base_fee = U256::from(base_fee.header.base_fee_per_gas.unwrap());

    let free = WithOtherFields::new(TransactionRequest::default().from(from).to(contract));
    let typed_free =
        WithOtherFields::new(free.inner.clone().max_fee_per_gas(0).max_priority_fee_per_gas(0));
    let priced = WithOtherFields::new(free.inner.clone().gas_price(base_fee.to()));
    let trace = || [TraceType::Trace].into_iter().collect();
    for (request, expected) in
        [(free.clone(), U256::ZERO), (typed_free, U256::ZERO), (priced.clone(), base_fee)]
    {
        let output = provider.call(request.clone()).block(BlockId::latest()).await.unwrap();
        assert_eq!(U256::from_be_slice(&output), expected);

        let traced = api.trace_call(request, trace(), Some(BlockId::latest())).await.unwrap();
        assert_eq!(U256::from_be_slice(&traced.output), expected);
    }

    // Like geth, access lists are built at the block's base fee. The contract loads slot BASEFEE.
    let sload_base_fee = Address::repeat_byte(0x54);
    api.anvil_set_code(sload_base_fee, bytes!("485400")).await.unwrap();
    for request in [free.clone(), priced.clone()] {
        let request = WithOtherFields::new(request.inner.to(sload_base_fee));
        let result = api.create_access_list(request, None, None).await.unwrap();
        assert_eq!(result.access_list.0[0].storage_keys, [B256::from(base_fee)]);
    }

    // Each call in a batch gets its own fee environment.
    let batch = [free.clone(), priced, free].map(|request| (request, trace()));
    let traced = api.trace_call_many(batch.to_vec(), Some(BlockId::latest())).await.unwrap();
    let outputs =
        traced.iter().map(|result| U256::from_be_slice(&result.output)).collect::<Vec<_>>();
    assert_eq!(outputs, [U256::ZERO, base_fee, U256::ZERO]);
}

#[tokio::test(flavor = "multi_thread")]
async fn priced_calls_below_base_fee_are_rejected() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();
    let to = handle.dev_accounts().nth(1).unwrap();

    // Returns GASPRICE.
    let contract = Address::repeat_byte(0x3a);
    api.anvil_set_code(contract, bytes!("3a5f5260205ff3")).await.unwrap();
    let block = provider.get_block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let base_fee = block.header.base_fee_per_gas.unwrap() as u128;
    let latest = Some(BlockId::latest());
    let trace = || [TraceType::Trace].into_iter().collect();
    let is_fee_cap_too_low = |err: BlockchainError| {
        matches!(err, BlockchainError::InvalidTransaction(InvalidTransactionError::FeeCapTooLow))
    };

    let call = TransactionRequest::default().from(from).to(contract);
    let legacy = WithOtherFields::new(call.clone().gas_price(base_fee - 1));
    let capped = WithOtherFields::new(call.clone().max_fee_per_gas(base_fee - 1));
    let transfer = WithOtherFields::new(
        TransactionRequest::default().from(from).to(to).gas_price(base_fee - 1),
    );
    for request in [legacy, capped, transfer] {
        let err = api.call(request.clone(), latest, Default::default()).await.unwrap_err();
        assert!(is_fee_cap_too_low(err));
        let err = api.trace_call(request.clone(), trace(), latest).await.unwrap_err();
        assert!(is_fee_cap_too_low(err));
        let err = api.estimate_gas(request.clone(), latest, Default::default()).await.unwrap_err();
        assert!(is_fee_cap_too_low(err));

        // A batch with an underpriced call fails as a whole.
        let free = WithOtherFields::new(call.clone());
        let batch = vec![(free, trace()), (request, trace())];
        let err = api.trace_call_many(batch, latest).await.unwrap_err();
        assert!(is_fee_cap_too_low(err));
    }

    // A fee cap at the base fee is accepted even when the tip cannot be paid in full.
    let tipped = WithOtherFields::new(call.max_fee_per_gas(base_fee).max_priority_fee_per_gas(1));
    let output = api.call(tipped.clone(), latest, Default::default()).await.unwrap();
    assert_eq!(U256::from_be_slice(&output), U256::from(base_fee));
    let traced = api.trace_call(tipped, trace(), latest).await.unwrap();
    assert_eq!(U256::from_be_slice(&traced.output), U256::from(base_fee));
}

#[tokio::test(flavor = "multi_thread")]
async fn priced_calls_below_base_fee_are_rejected_after_zero_base_fee() {
    let (api, handle) = spawn(NodeConfig::test().with_base_fee(Some(0))).await;
    let from = handle.dev_wallets().next().unwrap().address();
    let to = handle.dev_accounts().nth(1).unwrap();
    let call = WithOtherFields::new(TransactionRequest::default().from(from).to(to).gas_price(1));

    // A zero base fee disables the check for this call only.
    api.call(call.clone(), Some(BlockId::latest()), Default::default()).await.unwrap();

    api.anvil_set_next_block_base_fee_per_gas(U256::from(INITIAL_BASE_FEE)).await.unwrap();
    api.mine_one().await.unwrap();
    let err = api.call(call, Some(BlockId::latest()), Default::default()).await.unwrap_err();
    assert!(matches!(
        err,
        BlockchainError::InvalidTransaction(InvalidTransactionError::FeeCapTooLow)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn priced_calls_skip_base_fee_check_before_london() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Berlin.into()))).await;
    let from = handle.dev_wallets().next().unwrap().address();
    let to = handle.dev_accounts().nth(1).unwrap();
    let transfer =
        WithOtherFields::new(TransactionRequest::default().from(from).to(to).gas_price(1));
    let latest = Some(BlockId::latest());

    api.call(transfer.clone(), latest, Default::default()).await.unwrap();
    let gas = api.estimate_gas(transfer, latest, Default::default()).await.unwrap();
    assert_eq!(gas, U256::from(21_000));
}

// <https://github.com/foundry-rs/foundry/issues/17428>
#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas_transfers_across_hardforks() {
    for hardfork in [EthereumHardfork::Osaka, EthereumHardfork::Amsterdam] {
        let (api, handle) = spawn(NodeConfig::test().with_hardfork(Some(hardfork.into()))).await;
        let provider = handle.http_provider();
        let mut accounts = handle.dev_accounts();
        let from = accounts.next().unwrap();
        let existing = accounts.next().unwrap();
        let fresh = Address::random();
        // Bytecode excludes this call from the transfer shortcut, so the binary search runs.
        let contract = Address::random();
        api.anvil_set_code(contract, bytes!("00")).await.unwrap();
        let amsterdam = hardfork == EthereumHardfork::Amsterdam;

        // Under EIP-2780 only a value transfer to another account costs 21000. Under EIP-8037 a
        // value transfer to an empty account also pays new-account state gas.
        for (to, value, expected) in [
            (from, U256::ONE, if amsterdam { 12_000 } else { GAS_TRANSFER }),
            (existing, U256::ZERO, if amsterdam { 15_000 } else { GAS_TRANSFER }),
            (existing, U256::ONE, GAS_TRANSFER),
            (fresh, U256::ZERO, if amsterdam { 15_000 } else { GAS_TRANSFER }),
            (fresh, U256::ONE, if amsterdam { 204_600 } else { GAS_TRANSFER }),
            (contract, U256::ZERO, if amsterdam { 15_000 } else { GAS_TRANSFER }),
        ] {
            let tx = TransactionRequest::default().with_from(from).with_to(to).with_value(value);
            for gas_limit in [None, Some(expected)] {
                let mut tx = tx.clone();
                tx.gas = gas_limit;
                assert_eq!(
                    provider.estimate_gas(WithOtherFields::new(tx)).await.unwrap(),
                    expected
                );
            }
            let short = tx.clone().with_gas_limit(expected - 1);
            assert!(provider.estimate_gas(WithOtherFields::new(short)).await.is_err());

            let receipt = provider
                .send_transaction(WithOtherFields::new(tx.with_gas_limit(expected)))
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap();
            assert!(receipt.status());
            assert_eq!(receipt.gas_used, expected);
        }
    }
}
