//! tests for otterscan endpoints
use crate::{
    abi::{AlloyMulticallContract, MulticallContract},
    utils::{
        ethers_ws_provider, http_provider, http_provider_with_signer, ContractInstanceCompat,
        DeploymentTxFactoryCompat,
    },
};
use alloy_network::EthereumSigner;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions, TransactionRequest, WithOtherFields};
use anvil::{
    eth::otterscan::types::{
        OtsInternalOperation, OtsInternalOperationType, OtsTrace, OtsTraceType,
    },
    spawn, NodeConfig,
};
// use ethers::{
//     abi::Address,
//     prelude::{ContractFactory, ContractInstance, Middleware, SignerMiddleware},
//     signers::Signer,
//     types::{Bytes, TransactionRequest},
//     utils::get_contract_address,
// };
use foundry_common::types::{ToAlloy, ToEthers};
use foundry_compilers::{project_util::TempProject, Artifact};
use std::{collections::VecDeque, str::FromStr, sync::Arc};

#[tokio::test(flavor = "multi_thread")]
async fn can_call_erigon_get_header_by_number() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await;

    let res0 = api.erigon_get_header_by_number(0.into()).await.unwrap().unwrap();
    let res1 = api.erigon_get_header_by_number(1.into()).await.unwrap().unwrap();

    assert_eq!(res0.header.number, Some(0));
    assert_eq!(res1.header.number, Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_api_level() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    assert_eq!(api.ots_get_api_level().await.unwrap(), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_deploy() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let contract_receipt = AlloyMulticallContract::deploy_builder(provider.clone())
        .from(sender)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);

    let res = api.ots_get_internal_operations(contract_receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Create,
            from: sender,
            to: contract_address,
            value: U256::from(0)
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_transfer() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = http_provider(&handle.http_endpoint());

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Transfer,
            from,
            to,
            value: amount
        }
    );
}
