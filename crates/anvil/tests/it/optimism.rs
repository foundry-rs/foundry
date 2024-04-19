//! Tests for OP chain support.

use crate::utils::{http_provider, http_provider_with_signer};
use alloy_network::{EthereumSigner, TransactionBuilder};
use alloy_primitives::{address, bytes, fixed_bytes, U128, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{optimism::OptimismTransactionFields, TransactionRequest, WithOtherFields};
use anvil::{spawn, Hardfork, NodeConfig};
use anvil_core::eth::transaction::{
    optimism::{DepositTransaction, DepositTransactionRequest},
    to_alloy_transaction_with_hash_and_sender, transaction_request_to_typed, TypedTransaction,
    TypedTransactionRequest,
};
use std::str::FromStr;

#[tokio::test(flavor = "multi_thread")]
async fn test_deposits_not_supported_if_optimism_disabled() {
    // optimism disabled by default
    let (_, handle) = spawn(NodeConfig::test()).await;
    let provider = http_provider(&handle.http_endpoint());

    let from_addr = address!("cf7f9e66af820a19257a2108375b180b0ec49167");
    let to_addr = address!("71562b71999873db5b286df957af199ec94617f7");
    let deposit_tx = TypedTransaction::Deposit(DepositTransaction {
        from: from_addr,
        kind: to_addr.into(),
        value: U256::from(1234),
        gas_limit: 21000,
        input: bytes!(""),
        nonce: 0,
        source_hash: fixed_bytes!(
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        mint: U256::from(0),
        is_system_tx: true,
    });
    let deposit_tx_hash = deposit_tx.hash();
    let deposit_tx =
        to_alloy_transaction_with_hash_and_sender(deposit_tx, deposit_tx_hash, from_addr)
            .into_request();
    let deposit_tx = WithOtherFields::new(deposit_tx);

    let res = provider.send_transaction(deposit_tx).await.unwrap().register().await;
    assert!(res
        .unwrap_err()
        .to_string()
        .contains("op-stack deposit tx received but is not supported"));
}
