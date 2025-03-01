use crate::run_with_client;
use alloy_chains::Chain;
use alloy_primitives::U256;
use foundry_block_explorers::errors::EtherscanError;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn gas_estimate_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let result = client.gas_estimate(U256::from(2000000000u32)).await;

        result.unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn gas_estimate_error() {
    run_with_client(Chain::mainnet(), |client| async move {
        let err = client.gas_estimate(U256::from(7123189371829732819379218u128)).await.unwrap_err();

        assert!(matches!(err, EtherscanError::GasEstimationFailed));
    })
    .await
}

#[tokio::test]
#[serial]
async fn gas_oracle_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let result = client.gas_oracle().await;

        assert!(result.is_ok());

        let oracle = result.unwrap();

        assert!(oracle.safe_gas_price > U256::ZERO);
        assert!(oracle.propose_gas_price > U256::ZERO);
        assert!(oracle.fast_gas_price > U256::ZERO);
        assert!(oracle.last_block > 0);
        assert!(oracle.suggested_base_fee > U256::ZERO);
        assert!(!oracle.gas_used_ratio.is_empty());
    })
    .await
}
