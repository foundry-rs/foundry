//! genesis.json tests

use crate::fork::fork_config;
use alloy_genesis::Genesis;
use alloy_primitives::{Address, U256, address, b64, b256, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::BlockNumberOrTag;
use reth_anvil::{NodeConfig, spawn};

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

const GENESIS_HEADER: &str = r#"{
  "config": {
    "chainId": 19763,
    "homesteadBlock": 0,
    "eip150Block": 0,
    "eip155Block": 0,
    "eip158Block": 0,
    "byzantiumBlock": 0,
    "constantinopleBlock": 0,
    "petersburgBlock": 0,
    "istanbulBlock": 0,
    "berlinBlock": 0,
    "londonBlock": 0,
    "terminalTotalDifficulty": 0,
    "mergeNetsplitBlock": 0,
    "ethash": {}
  },
  "nonce": "0x42",
  "timestamp": "0x123",
  "extraData": "0x1234",
  "gasLimit": "0x989680",
  "difficulty": "0x20000",
  "mixHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
  "coinbase": "0x2222222222222222222222222222222222222222",
  "alloc": {
    "3333333333333333333333333333333333333333": {
      "balance": "0x1",
      "nonce": "0x2",
      "code": "0x6000"
    }
  },
  "baseFeePerGas": "0x7"
}
"#;

#[tokio::test(flavor = "multi_thread")]
async fn can_apply_genesis() {
    let genesis: Genesis = serde_json::from_str(GENESIS).unwrap();
    let (_api, handle) = spawn(NodeConfig::test().with_genesis(Some(genesis))).await;

    let provider = handle.http_provider();

    assert_eq!(provider.get_chain_id().await.unwrap(), 19763u64);

    let addr: Address = address!("71562b71999873db5b286df957af199ec94617f7");
    let balance = provider.get_balance(addr).await.unwrap();

    let expected: U256 = U256::from_str_radix("ffffffffffffffffffffffffff", 16).unwrap();
    assert_eq!(balance, expected);

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, 73u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn applies_genesis_header() {
    let genesis: Genesis = serde_json::from_str(GENESIS_HEADER).unwrap();
    let (_api, handle) = spawn(NodeConfig::test().with_genesis(Some(genesis))).await;

    let block = handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Earliest)
        .await
        .unwrap()
        .unwrap();

    // Anvil's header carries the forks of the genesis config and the state root of its alloc
    // alone, so its hash differs; the header fields the genesis sets are the same.
    assert_eq!(block.header.nonce, Some(b64!("0x0000000000000042")));
    assert_eq!(block.header.timestamp, 0x123);
    assert_eq!(block.header.extra_data, bytes!("0x1234"));
    assert_eq!(block.header.gas_limit, 0x989680);
    assert_eq!(block.header.difficulty, U256::from(0x20000));
    assert_eq!(
        block.header.mix_hash,
        Some(b256!("0x1111111111111111111111111111111111111111111111111111111111111111"))
    );
    assert_eq!(block.header.beneficiary, address!("0x2222222222222222222222222222222222222222"));
    assert_eq!(block.header.base_fee_per_gas, Some(7));
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
