//! general eth api tests

use crate::abi::Greeter;
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Bytes, U256, Uint, address, b256, utils::Unit};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, eth::backend::db::SerializableState, spawn};
use foundry_test_utils::rpc::next_http_archive_rpc_url;
use revm::{
    context_interface::block::BlobExcessGasAndPrice,
    primitives::eip4844::BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE,
};
use serde_json::json;
use std::str::FromStr;

#[tokio::test(flavor = "multi_thread")]
async fn can_load_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, _handle) = spawn(NodeConfig::test()).await;

    api.mine_one().await;
    api.mine_one().await;

    let num = api.block_number().unwrap();

    let state = api.serialized_state(false).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &state).unwrap();

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let num2 = api.block_number().unwrap();

    // Ref: https://github.com/foundry-rs/foundry/issues/9017
    // Check responses of eth_blockNumber and eth_getBlockByNumber don't deviate after loading state
    let num_from_tag = api
        .block_by_number(alloy_eips::BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .number;
    assert_eq!(num, num2);

    assert_eq!(num, U256::from(num_from_tag));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state_legacy() {
    let state_file = "test-data/state-dump-legacy.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state_legacy_stress() {
    let state_file = "test-data/state-dump-legacy-stress.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_load_existing_state() {
    let state_file = "test-data/state-dump.json";

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_make_sure_historical_state_is_not_cleared_on_dump() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let greeter = Greeter::deploy(&provider, "Hello".to_string()).await.unwrap();

    let address = greeter.address();

    let _tx = greeter
        .setGreeting("World!".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    api.mine_one().await;

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(3));

    // Makes sure historical states of the new instance are not cleared.
    let code = provider.get_code_at(*address).block_id(BlockId::number(2)).await.unwrap();

    assert_ne!(code, Bytes::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_preserve_historical_states_between_dump_and_load() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let greeter = Greeter::deploy(&provider, "Hello".to_string()).await.unwrap();

    let address = greeter.address();

    let deploy_blk_num = provider.get_block_number().await.unwrap();

    let tx = greeter
        .setGreeting("World!".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let change_greeting_blk_num = tx.block_number.unwrap();

    api.mine_one().await;

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let (api, handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;

    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, Uint::from(3));

    let provider = handle.http_provider();

    let greeter = Greeter::new(*address, provider);

    let greeting_at_init =
        greeter.greet().block(BlockId::number(deploy_blk_num)).call().await.unwrap();

    assert_eq!(greeting_at_init, "Hello");

    let greeting_after_change =
        greeter.greet().block(BlockId::number(change_greeting_blk_num)).call().await.unwrap();

    assert_eq!(greeting_after_change, "World!");
}

// <https://github.com/foundry-rs/foundry/issues/9053>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_load_state() {
    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070682u64)),
    )
    .await;

    let bob = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let alice = address!("0x9276449EaC5b4f7Bc17cFC6700f7BeeB86F9bCd0");

    let provider = handle.http_provider();

    let init_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let init_balance_alice = provider.get_balance(alice).await.unwrap();

    let value = Unit::ETHER.wei().saturating_mul(U256::from(1)); // 1 ether
    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let serialized_state = api.serialized_state(false).await.unwrap();

    let state_dump_block = api.block_number().unwrap();

    let (api, handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070686u64)) // Forked chain has moved forward
            .with_init_state(Some(serialized_state)),
    )
    .await;

    // Ensure the initial block number is the fork_block_number and not the state_dump_block
    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, U256::from(21070686u64));
    assert_ne!(block_number, state_dump_block);

    let provider = handle.http_provider();

    let restart_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let restart_balance_alice = provider.get_balance(alice).await.unwrap();

    assert_eq!(init_nonce_bob + 1, restart_nonce_bob);

    assert_eq!(init_balance_alice + value, restart_balance_alice);

    // Send another tx to check if the state is preserved

    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let balance_alice = provider.get_balance(alice).await.unwrap();

    let tx = TransactionRequest::default()
        .with_to(alice)
        .with_value(value)
        .with_from(bob)
        .with_nonce(nonce_bob);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let latest_nonce_bob = provider.get_transaction_count(bob).await.unwrap();

    let latest_balance_alice = provider.get_balance(alice).await.unwrap();

    assert_eq!(nonce_bob + 1, latest_nonce_bob);

    assert_eq!(balance_alice + value, latest_balance_alice);
}

// <https://github.com/foundry-rs/foundry/issues/9539>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_load_state_with_greater_state_block() {
    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070682u64)),
    )
    .await;

    api.mine_one().await;

    let block_number = api.block_number().unwrap();

    let serialized_state = api.serialized_state(false).await.unwrap();

    assert_eq!(serialized_state.best_block_number, Some(block_number.to::<u64>()));

    let (api, _handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
            .with_fork_block_number(Some(21070682u64)) // Forked chain has moved forward
            .with_init_state(Some(serialized_state)),
    )
    .await;

    let new_block_number = api.block_number().unwrap();

    assert_eq!(new_block_number, block_number);
}

// <https://github.com/foundry-rs/foundry/issues/10488>
#[tokio::test(flavor = "multi_thread")]
async fn computes_next_base_fee_after_loading_state() {
    let tmp = tempfile::tempdir().unwrap();
    let state_file = tmp.path().join("state.json");

    let (api, handle) = spawn(NodeConfig::test()).await;

    let bob = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    let alice = address!("0x9276449EaC5b4f7Bc17cFC6700f7BeeB86F9bCd0");

    let provider = handle.http_provider();

    let base_fee_empty_chain = api.backend.fees().base_fee();

    let value = Unit::ETHER.wei().saturating_mul(U256::from(1)); // 1 ether
    let tx = TransactionRequest::default().with_to(alice).with_value(value).with_from(bob);
    let tx = WithOtherFields::new(tx);

    let _receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let base_fee_after_one_tx = api.backend.fees().base_fee();
    // the test is meaningless if this does not hold
    assert_ne!(base_fee_empty_chain, base_fee_after_one_tx);

    let ser_state = api.serialized_state(true).await.unwrap();
    foundry_common::fs::write_json_file(&state_file, &ser_state).unwrap();

    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(state_file)).await;
    let base_fee_after_reload = api.backend.fees().base_fee();
    assert_eq!(base_fee_after_reload, base_fee_after_one_tx);
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_deserialization_v1_2() {
    let old_format = r#"{
        "block": {
            "number": "0x5",
            "coinbase": "0x1234567890123456789012345678901234567890",
            "timestamp": "0x688c83b5",
            "gas_limit": "0x1c9c380",
            "basefee": "0x3b9aca00",  
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
            "blob_excess_gas_and_price": {
                "excess_blob_gas": 0,
                "blob_gasprice": 1
            }
        },
        "accounts": {},
        "best_block_number": "0x5",
        "blocks": [],
        "transactions": []
    }"#;

    let state: SerializableState = serde_json::from_str(old_format).unwrap();
    assert!(state.block.is_some());
    let block_env = state.block.unwrap();
    assert_eq!(block_env.number, U256::from(5));
    // Verify coinbase was converted to beneficiary
    assert_eq!(block_env.beneficiary, address!("0x1234567890123456789012345678901234567890"));

    // New format with beneficiary and numeric values
    let new_format = r#"{
        "block": {
            "number": 6,
            "beneficiary": "0x1234567890123456789012345678901234567891",
            "timestamp": 1751619509,
            "gas_limit": 30000000,
            "basefee": 1000000000,
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
            "blob_excess_gas_and_price": {
                "excess_blob_gas": 0,
                "blob_gasprice": 1
            }
        },
        "accounts": {},
        "best_block_number": 6,
        "blocks": [],
        "transactions": []
    }"#;

    let state: SerializableState = serde_json::from_str(new_format).unwrap();
    assert!(state.block.is_some());
    let block_env = state.block.unwrap();
    assert_eq!(block_env.number, U256::from(6));
    assert_eq!(block_env.beneficiary, address!("0x1234567890123456789012345678901234567891"));
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_mixed_formats_deserialization_v1_2() {
    let mixed_format = json!({
        "block": {
            "number": "0x3",
            "coinbase": "0x1111111111111111111111111111111111111111",
            "timestamp": 1751619509,
            "gas_limit": "0x1c9c380",
            "basefee": 1000000000,
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
            "blob_excess_gas_and_price": {
                "excess_blob_gas": 0,
                "blob_gasprice": 1
            }
        },
        "accounts": {},
        "best_block_number": 3,
        "blocks": [],
        "transactions": []
    });

    let state: SerializableState = serde_json::from_str(&mixed_format.to_string()).unwrap();
    let block_env = state.block.unwrap();

    assert_eq!(block_env.number, U256::from(3));
    assert_eq!(block_env.beneficiary, address!("0x1111111111111111111111111111111111111111"));
    assert_eq!(block_env.timestamp, U256::from(1751619509));
    assert_eq!(block_env.gas_limit, 0x1c9c380);
    assert_eq!(block_env.basefee, 1_000_000_000);
    assert_eq!(block_env.difficulty, U256::ZERO);
    assert_eq!(
        block_env.prevrandao.unwrap(),
        b256!("ecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5")
    );

    let blob = block_env.blob_excess_gas_and_price.unwrap();
    assert_eq!(blob.excess_blob_gas, 0);
    assert_eq!(blob.blob_gasprice, 1);

    assert_eq!(state.best_block_number, Some(3));
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_optional_fields_deserialization_v1_2() {
    let partial_old_format = json!({
        "block": {
            "number": "0x1",
            "coinbase": "0x0000000000000000000000000000000000000000",
            "timestamp": "0x688c83b5",
            "gas_limit": "0x1c9c380",
            "basefee": "0x3b9aca00",
            "difficulty": "0x0",
            "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5"
            // Missing blob_excess_gas_and_price - should be None
        },
        "accounts": {},
        "best_block_number": "0x1"
        // Missing blocks and transactions arrays - should default to empty
    });

    let state: SerializableState = serde_json::from_str(&partial_old_format.to_string()).unwrap();

    let block_env = state.block.unwrap();
    assert_eq!(block_env.number, U256::from(1));
    assert_eq!(block_env.beneficiary, address!("0x0000000000000000000000000000000000000000"));
    assert_eq!(block_env.timestamp, U256::from(0x688c83b5));
    assert_eq!(block_env.gas_limit, 0x1c9c380);
    assert_eq!(block_env.basefee, 0x3b9aca00);
    assert_eq!(block_env.difficulty, U256::ZERO);
    assert_eq!(
        block_env.prevrandao.unwrap(),
        b256!("ecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5")
    );
    assert_eq!(
        block_env.blob_excess_gas_and_price,
        Some(BlobExcessGasAndPrice::new(0, BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE))
    );

    assert_eq!(state.best_block_number, Some(1));
    assert!(state.blocks.is_empty());
    assert!(state.transactions.is_empty());
}

// <https://github.com/foundry-rs/foundry/issues/11176>
#[tokio::test(flavor = "multi_thread")]
async fn test_backward_compatibility_state_dump_deserialization_v1_2() {
    let tmp = tempfile::tempdir().unwrap();
    let old_state_file = tmp.path().join("old_state.json");

    // A simple state dump with a single block containing one transaction of a Counter contract
    // deployment.
    let old_state_json = json!({
      "block": {
        "number": "0x1",
        "coinbase": "0x0000000000000000000000000000000000000001",
        "timestamp": "0x688c83b5",
        "gas_limit": "0x1c9c380",
        "basefee": "0x3b9aca00",
        "difficulty": "0x0",
        "prevrandao": "0xecc5f0af8ff6b65c14bfdac55ba9db870d89482eb2b87200c6d7e7cd3a3a5ad5",
        "blob_excess_gas_and_price": {
          "excess_blob_gas": 0,
          "blob_gasprice": 1
        }
      },
      "accounts": {
        "0x0000000000000000000000000000000000000000": {
          "nonce": 0,
          "balance": "0x26481",
          "code": "0x",
          "storage": {}
        },
        "0x14dc79964da2c08b23698b3d3cc7ca32193d9955": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x23618e81e3f5cdf7f54c3d65f7fbc0abf5b21e8f": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x4e59b44847b379578588920ca78fbf26c0b4956c": {
          "nonce": 0,
          "balance": "0x0",
          "code": "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3",
          "storage": {}
        },
        "0x5fbdb2315678afecb367f032d93f642f64180aa3": {
          "nonce": 1,
          "balance": "0x0",
          "code": "0x608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
          "storage": {}
        },
        "0x70997970c51812dc3a010c7d01b50e0d17dc79c8": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x90f79bf6eb2c4f870365e785982e1f101e93b906": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x976ea74026e726554db657fa54763abd0c3a0aa9": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0x9965507d1a55bcc2695c58ba16fb37d819b0a4dc": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0xa0ee7a142d267c1f36714e4a8f75612f20a79720": {
          "nonce": 0,
          "balance": "0x21e19e0c9bab2400000",
          "code": "0x",
          "storage": {}
        },
        "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266": {
          "nonce": 1,
          "balance": "0x21e19e03b1e9e55d17f",
          "code": "0x",
          "storage": {}
        }
      },
      "best_block_number": "0x1",
      "blocks": [
        {
          "header": {
            "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "miner": "0x0000000000000000000000000000000000000000",
            "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "receiptsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "difficulty": "0x0",
            "number": "0x0",
            "gasLimit": "0x1c9c380",
            "gasUsed": "0x0",
            "timestamp": "0x688c83b0",
            "extraData": "0x",
            "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "nonce": "0x0000000000000000",
            "baseFeePerGas": "0x3b9aca00",
            "withdrawalsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "blobGasUsed": "0x0",
            "excessBlobGas": "0x0",
            "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000"
          },
          "transactions": [],
          "ommers": []
        },
        {
          "header": {
            "parentHash": "0x25097583380d90c4ac42b454ed7d2f59450ed3a16fdcf7f7bd93295aa126a901",
            "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "miner": "0x0000000000000000000000000000000000000000",
            "stateRoot": "0x6e005b459ac9acefa5f47fd2d7ff8ca81a91794fdc5f7fbc3e2faeeaefe5d516",
            "transactionsRoot": "0x59f0457ec18e2181c186f49d9ac911b33b5f4f55db5c494022147346bcfc9837",
            "receiptsRoot": "0x88ac48b910f796aab7407814203b3a15a04a812f387e92efeccc92a2ecf809da",
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "difficulty": "0x0",
            "number": "0x1",
            "gasLimit": "0x1c9c380",
            "gasUsed": "0x26481",
            "timestamp": "0x688c83b5",
            "extraData": "0x",
            "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "nonce": "0x0000000000000000",
            "baseFeePerGas": "0x3b9aca00",
            "withdrawalsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "blobGasUsed": "0x0",
            "excessBlobGas": "0x0",
            "parentBeaconBlockRoot": "0x0000000000000000000000000000000000000000000000000000000000000000"
          },
          "transactions": [
            {
              "transaction": {
                "EIP1559": {
                  "chainId": "0x7a69",
                  "nonce": "0x0",
                  "gas": "0x31c41",
                  "maxFeePerGas": "0x77359401",
                  "maxPriorityFeePerGas": "0x1",
                  "to": null,
                  "value": "0x0",
                  "accessList": [],
                  "input": "0x6080604052348015600e575f5ffd5b506101e18061001c5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
                  "r": "0xa7398e28ca9a56b423cab87aeb3612378bac9c5684aaf778a78943f2637fd731",
                  "s": "0x583511da658f564253c8c0f9ee1820ef370f23556be504b304ac1292f869d9a0",
                  "yParity": "0x0",
                  "v": "0x0",
                  "hash": "0x9e4846328caa09cbe8086d11b7e115adf70390e79ff203d8e5f37785c2a890be"
                }
              },
              "impersonated_sender": null
            }
          ],
          "ommers": []
        }
      ],
      "transactions": [
        {
          "info": {
            "transaction_hash": "0x9e4846328caa09cbe8086d11b7e115adf70390e79ff203d8e5f37785c2a890be",
            "transaction_index": 0,
            "from": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "to": null,
            "contract_address": "0x5fbdb2315678afecb367f032d93f642f64180aa3",
            "traces": [
              {
                "parent": null,
                "children": [],
                "idx": 0,
                "trace": {
                  "depth": 0,
                  "success": true,
                  "caller": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                  "address": "0x5fbdb2315678afecb367f032d93f642f64180aa3",
                  "maybe_precompile": false,
                  "selfdestruct_address": null,
                  "selfdestruct_refund_target": null,
                  "selfdestruct_transferred_value": null,
                  "kind": "CREATE",
                  "value": "0x0",
                  "data": "0x6080604052348015600e575f5ffd5b506101e18061001c5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
                  "output": "0x608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
                  "gas_used": 96345,
                  "gas_limit": 143385,
                  "status": "Return",
                  "steps": [],
                  "decoded": {
                    "label": null,
                    "return_data": null,
                    "call_data": null
                  }
                },
                "logs": [],
                "ordering": []
              }
            ],
            "exit": "Return",
            "out": "0x608060405234801561000f575f5ffd5b506004361061003f575f3560e01c80633fb5c1cb146100435780638381f58a1461005f578063d09de08a1461007d575b5f5ffd5b61005d600480360381019061005891906100e4565b610087565b005b610067610090565b604051610074919061011e565b60405180910390f35b610085610095565b005b805f8190555050565b5f5481565b5f5f8154809291906100a690610164565b9190505550565b5f5ffd5b5f819050919050565b6100c3816100b1565b81146100cd575f5ffd5b50565b5f813590506100de816100ba565b92915050565b5f602082840312156100f9576100f86100ad565b5b5f610106848285016100d0565b91505092915050565b610118816100b1565b82525050565b5f6020820190506101315f83018461010f565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61016e826100b1565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82036101a05761019f610137565b5b60018201905091905056fea264697066735822122040b6a3cd3ec8f890002f39a8719ebee029ba9bac3d7fa9d581d4712cfe9ffec264736f6c634300081e0033",
            "nonce": 0,
            "gas_used": 156801
          },
          "receipt": {
            "type": "0x2",
            "status": "0x1",
            "cumulativeGasUsed": "0x26481",
            "logs": [],
            "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
          },
          "block_hash": "0x313ea0d32d662434a55a20d7c58544e6baaea421b6eccf4b68392dec2a76d771",
          "block_number": 1
        }
      ],
      "historical_states": null
    });

    // Write the old state to file.
    foundry_common::fs::write_json_file(&old_state_file, &old_state_json).unwrap();

    // Test deserializing the old state dump directly.
    let deserialized_state: SerializableState = serde_json::from_value(old_state_json).unwrap();

    // Verify the old state was loaded correctly with `coinbase` to `beneficiary` conversion.
    let block_env = deserialized_state.block.unwrap();
    assert_eq!(block_env.number, U256::from(1));
    assert_eq!(block_env.beneficiary, address!("0000000000000000000000000000000000000001"));
    assert_eq!(block_env.gas_limit, 0x1c9c380);
    assert_eq!(block_env.basefee, 0x3b9aca00);

    // Verify best_block_number hex string parsing.
    assert_eq!(deserialized_state.best_block_number, Some(1));

    // Verify account data was preserved.
    assert_eq!(deserialized_state.accounts.len(), 13);

    // Test specific accounts from the old dump.
    let deployer_addr = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap();
    let deployer_account = deserialized_state.accounts.get(&deployer_addr).unwrap();
    assert_eq!(deployer_account.nonce, 1);
    assert_eq!(deployer_account.balance, U256::from_str("0x21e19e03b1e9e55d17f").unwrap());

    // Test contract account.
    let contract_addr = "0x5fbdb2315678afecb367f032d93f642f64180aa3".parse().unwrap();
    let contract_account = deserialized_state.accounts.get(&contract_addr).unwrap();
    assert_eq!(contract_account.nonce, 1);
    assert_eq!(contract_account.balance, U256::ZERO);
    assert!(!contract_account.code.is_empty());

    // Verify blocks and transactions are preserved.
    assert_eq!(deserialized_state.blocks.len(), 2);
    assert_eq!(deserialized_state.transactions.len(), 1);

    // Test that Anvil can load this old state dump.
    let (api, _handle) = spawn(NodeConfig::test().with_init_state_path(&old_state_file)).await;

    // Verify the state was loaded correctly.
    let block_number = api.block_number().unwrap();
    assert_eq!(block_number, U256::from(1));

    // Verify account balances are preserved.
    let provider = _handle.http_provider();
    let deployer_balance = provider.get_balance(deployer_addr).await.unwrap();
    assert_eq!(deployer_balance, U256::from_str("0x21e19e03b1e9e55d17f").unwrap());
    let contract_balance = provider.get_balance(contract_addr).await.unwrap();
    assert_eq!(contract_balance, U256::ZERO);

    // Verify contract code is preserved.
    let contract_code = provider.get_code_at(contract_addr).await.unwrap();
    assert!(!contract_code.is_empty());
}
