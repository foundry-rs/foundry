//! genesis.json tests

use crate::fork::fork_config;
use alloy_genesis::Genesis;
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use anvil::{NodeConfig, spawn};
use std::str::FromStr;

const GENESIS: &str = r#"{
  "config": {
    "chainId": 19763,
    "homesteadBlock": 0,
    "eip150Block": 0,
    "eip155Block": 0,
    "eip158Block": 0,
    "byzantiumBlock": 0,
    "ethash": {}
  },
  "nonce": "0xdeadbeefdeadbeef",
  "timestamp": "0x0",
  "extraData": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "gasLimit": "0x80000000",
  "difficulty": "0x20000",
  "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "coinbase": "0x0000000000000000000000000000000000000000",
  "alloc": {
    "71562b71999873db5b286df957af199ec94617f7": {
      "balance": "0xffffffffffffffffffffffffff"
    }
  },
  "number": 73,
  "gasUsed": "0x0",
  "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
}
"#;

#[tokio::test(flavor = "multi_thread")]
async fn can_apply_genesis() {
    let genesis: Genesis = serde_json::from_str(GENESIS).unwrap();
    let (_api, handle) = spawn(NodeConfig::test().with_genesis(Some(genesis))).await;

    let provider = handle.http_provider();

    assert_eq!(provider.get_chain_id().await.unwrap(), 19763u64);

    let addr: Address = Address::from_str("71562b71999873db5b286df957af199ec94617f7").unwrap();
    let balance = provider.get_balance(addr).await.unwrap();

    let expected: U256 = U256::from_str_radix("ffffffffffffffffffffffffff", 16).unwrap();
    assert_eq!(balance, expected);

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, 73u64);
}

// <https://github.com/foundry-rs/foundry/issues/10059>
// <https://github.com/foundry-rs/foundry/issues/10238>
#[tokio::test(flavor = "multi_thread")]
async fn chain_id_precedence() {
    // Order: --chain-id > fork-chain-id > Genesis > default.

    // --chain-id > Genesis.
    let genesis: Genesis = serde_json::from_str(GENESIS).unwrap();
    let (_api, handle) =
        spawn(NodeConfig::test().with_genesis(Some(genesis.clone())).with_chain_id(Some(300u64)))
            .await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 300u64);

    // fork > Genesis.
    let (_api, handle) = spawn(fork_config().with_genesis(Some(genesis.clone()))).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 1);

    // --chain-id > fork.
    let (_api, handle) = spawn(fork_config().with_chain_id(Some(300u64))).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 300u64);

    // fork
    let (_api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 1);

    // Genesis
    let (_api, handle) = spawn(NodeConfig::test().with_genesis(Some(genesis))).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 19763u64);

    // default
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 31337);
}
