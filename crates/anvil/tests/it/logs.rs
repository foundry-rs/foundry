//! log/event related tests

use crate::{
    abi::AlloySimpleStorage::{self, AlloySimpleStorageEvents},
    utils::http_provider_with_signer,
};
use alloy_network::EthereumSigner;
use alloy_primitives::B256;
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, Filter};
use anvil::{spawn, NodeConfig};
// use ethers::{
//     middleware::SignerMiddleware,
//     prelude::{BlockNumber, Filter, FilterKind, Middleware, Signer, H256},
//     types::Log,
// };
use foundry_common::types::ToEthers;
use futures::StreamExt;
use std::{str::FromStr, sync::Arc};

#[tokio::test(flavor = "multi_thread")]
async fn get_past_events() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let account = wallet.address();
    let signer: EthereumSigner = wallet.into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let simple_storage_contract =
        AlloySimpleStorage::deploy(provider.clone(), "initial value".to_string()).await.unwrap();
    let _ = simple_storage_contract
        .setValue("hi".to_string())
        .from(account)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let simple_storage_address = *simple_storage_contract.address();

    let filter = Filter::new()
        .address(simple_storage_address)
        .topic1(B256::from(account.into_word()))
        .from_block(BlockNumberOrTag::from(0));

    let logs = provider
        .get_logs(&filter)
        .await
        .unwrap()
        .into_iter()
        .map(|log| log.log_decode::<AlloySimpleStorage::ValueChanged>().unwrap())
        .collect::<Vec<_>>();

    // 2 events, 1 in constructor, 1 in call
    assert_eq!(logs[0].inner.newValue, "initial value");
    assert_eq!(logs[1].inner.newValue, "hi");
    assert_eq!(logs.len(), 2);

    // and we can fetch the events at a block hash
    // let hash = provider.get_block(1).await.unwrap().unwrap().hash.unwrap();
    let hash = provider
        .get_block_by_number(BlockNumberOrTag::from(1), false)
        .await
        .unwrap()
        .unwrap()
        .header
        .hash
        .unwrap();

    let filter = Filter::new()
        .address(simple_storage_address)
        .topic1(B256::from(account.into_word()))
        .at_block_hash(hash);

    let logs = provider
        .get_logs(&filter)
        .await
        .unwrap()
        .into_iter()
        .map(|log| log.log_decode::<AlloySimpleStorage::ValueChanged>().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(logs[0].inner.newValue, "initial value");
    assert_eq!(logs.len(), 1);
}
