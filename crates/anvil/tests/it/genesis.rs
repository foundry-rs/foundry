//! genesis.json tests

use alloy_genesis::Genesis;
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use anvil::{spawn, NodeConfig};
use std::str::FromStr;

#[tokio::test(flavor = "multi_thread")]
async fn can_apply_genesis() {
    let genesis = r#"{
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
  "number": "0x0",
  "gasUsed": "0x0",
  "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
}
"#;
    let genesis: Genesis = serde_json::from_str(genesis).unwrap();
    let (_api, handle) = spawn(NodeConfig::test().with_genesis(Some(genesis))).await;

    let provider = handle.http_provider();

    assert_eq!(provider.get_chain_id().await.unwrap(), 19763u64);

    let addr: Address = Address::from_str("71562b71999873db5b286df957af199ec94617f7").unwrap();
    let balance = provider.get_balance(addr).await.unwrap();

    let expected: U256 = U256::from_str_radix("ffffffffffffffffffffffffff", 16).unwrap();
    assert_eq!(balance, expected);
}
