//! Arc network tests.

use crate::utils::http_provider_with_signer;
use alloy_network::{EthereumWallet, ReceiptResponse, TransactionBuilder};
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use anvil::{NodeConfig, spawn};

#[tokio::test(flavor = "multi_thread")]
async fn arc_standalone_runs_arc_execution() {
    let (_api, handle) = spawn(NodeConfig::test_arc()).await;
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let (from, to) = (accounts[0].address(), accounts[1].address());
    let signer: EthereumWallet = accounts[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);
    let before = provider.get_balance(to).await.unwrap();

    let receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(from)
                .with_to(to)
                .with_value(U256::from(1_234))
                .with_gas_limit(21_000)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());
    assert_eq!(provider.get_balance(to).await.unwrap(), before + U256::from(1_234));
    // Arc logs native value transfers as EIP-7708 `Transfer` events, which Ethereum does not.
    let logs = receipt.inner.logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].address(), evm2::evm::SYSTEM_ADDRESS);
}
