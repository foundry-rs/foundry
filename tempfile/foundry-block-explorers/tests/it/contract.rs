use crate::{init_tracing, run_with_client, run_with_client_cached};
use alloy_chains::{Chain, NamedChain};
use foundry_block_explorers::{contract::SourceCodeMetadata, errors::EtherscanError, Client};
use serial_test::serial;

/// Abi of [0x00000000219ab540356cBB839Cbe05303d7705Fa](https://api.etherscan.io/api?module=contract&action=getsourcecode&address=0x00000000219ab540356cBB839Cbe05303d7705Fa).
const DEPOSIT_CONTRACT_ABI: &str = include!("../../test-data/deposit_contract.expr");

#[tokio::test]
#[serial]
async fn can_fetch_ftm_contract_abi() {
    run_with_client(Chain::from_named(NamedChain::Fantom), |client| async move {
        let _abi = client
            .contract_abi("0x80AA7cb0006d5DDD91cce684229Ac6e398864606".parse().unwrap())
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_fetch_contract_abi() {
    run_with_client(Chain::mainnet(), |client| async move {
        let abi = client
            .contract_abi("0x00000000219ab540356cBB839Cbe05303d7705Fa".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(abi, serde_json::from_str(DEPOSIT_CONTRACT_ABI).unwrap());
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_fetch_and_cache_contract_abi() {
    async fn fetch_abi(client: &Client, addr: &str) {
        let abi = client.contract_abi(addr.parse().unwrap()).await.unwrap();
        assert_eq!(abi, serde_json::from_str(DEPOSIT_CONTRACT_ABI).unwrap());
    }

    run_with_client_cached(Chain::mainnet(), |client| async move {
        // Fetch the abi and cache it.
        fetch_abi(&client, "0x00000000219ab540356cBB839Cbe05303d7705Fa").await;

        // Repeated calls on the cached abi should not trigger a new request.
        for _ in 0..10 {
            fetch_abi(&client, "0x00000000219ab540356cBB839Cbe05303d7705Fa").await;
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_fetch_and_cache_contract_source_code() {
    async fn fetch_source_code(client: &Client, addr: &str) {
        let meta = client.contract_source_code(addr.parse().unwrap()).await.unwrap();
        assert_eq!(meta.items.len(), 1);

        let item = &meta.items[0];
        assert!(matches!(item.source_code, SourceCodeMetadata::SourceCode(_)));
        assert_eq!(item.source_code.sources().len(), 1);
        assert_eq!(item.abi().unwrap(), serde_json::from_str(DEPOSIT_CONTRACT_ABI).unwrap());
    }

    run_with_client_cached(Chain::mainnet(), |client| async move {
        // Fetch the source code and cache it.
        fetch_source_code(&client, "0x00000000219ab540356cBB839Cbe05303d7705Fa").await;

        // Repeated calls on the cached source code should not trigger a new request.
        for _ in 0..10 {
            fetch_source_code(&client, "0x00000000219ab540356cBB839Cbe05303d7705Fa").await;
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn can_fetch_deposit_contract_source_code_from_blockscout() {
    let client = Client::builder()
        .with_url("https://eth.blockscout.com")
        .unwrap()
        .with_api_url("https://eth.blockscout.com/api")
        .unwrap()
        .with_api_key("test")
        .build()
        .unwrap();
    let meta = client
        .contract_source_code("0x00000000219ab540356cBB839Cbe05303d7705Fa".parse().unwrap())
        .await
        .unwrap();

    assert_eq!(meta.items.len(), 1);
    let item = &meta.items[0];
    assert!(matches!(item.source_code, SourceCodeMetadata::SourceCode(_)));
    assert_eq!(item.source_code.sources().len(), 1);
    assert_eq!(item.abi().unwrap(), serde_json::from_str(DEPOSIT_CONTRACT_ABI).unwrap());
}

#[tokio::test]
#[serial]
async fn can_fetch_other_contract_source_code_from_blockscout() {
    let client = Client::builder()
        .with_url("https://eth.blockscout.com")
        .unwrap()
        .with_api_url("https://eth.blockscout.com/api")
        .unwrap()
        .with_api_key("test")
        .build()
        .unwrap();
    let meta = client
        .contract_source_code("0xDef1C0ded9bec7F1a1670819833240f027b25EfF".parse().unwrap())
        .await
        .unwrap();

    assert_eq!(meta.items.len(), 1);
    let item = &meta.items[0];
    assert!(matches!(item.source_code, SourceCodeMetadata::SourceCode(_)));
    assert_eq!(item.source_code.sources().len(), 1);
}

#[tokio::test]
#[serial]
async fn can_fetch_contract_source_code() {
    run_with_client(Chain::mainnet(), |client| async move {
        let meta = client
            .contract_source_code("0x00000000219ab540356cBB839Cbe05303d7705Fa".parse().unwrap())
            .await
            .unwrap();

        assert_eq!(meta.items.len(), 1);
        let item = &meta.items[0];
        assert!(matches!(item.source_code, SourceCodeMetadata::SourceCode(_)));
        assert_eq!(item.source_code.sources().len(), 1);
        assert_eq!(item.abi().unwrap(), serde_json::from_str(DEPOSIT_CONTRACT_ABI).unwrap());
    })
    .await
}

#[tokio::test]
#[serial]
async fn can_get_error_on_unverified_contract() {
    init_tracing();
    run_with_client(Chain::mainnet(), |client| async move {
        let addr = "0xb5c31a0e22cae98ac08233e512bd627885aa24e5".parse().unwrap();
        let err = client.contract_source_code(addr).await.unwrap_err();
        assert!(matches!(err, EtherscanError::ContractCodeNotVerified(_)));
    })
    .await
}

/// Query a contract that has a single string source entry instead of underlying JSON metadata.
#[tokio::test]
#[serial]
async fn can_fetch_contract_source_tree_for_singleton_contract() {
    run_with_client(Chain::mainnet(), |client| async move {
        let meta = client
            .contract_source_code("0x00000000219ab540356cBB839Cbe05303d7705Fa".parse().unwrap())
            .await
            .unwrap();

        assert_eq!(meta.items.len(), 1);
        let item = &meta.items[0];
        assert!(matches!(item.source_code, SourceCodeMetadata::SourceCode(_)));
        assert_eq!(item.source_code.sources().len(), 1);
        assert_eq!(item.abi().unwrap(), serde_json::from_str(DEPOSIT_CONTRACT_ABI).unwrap());
    })
    .await
}

/// Query a contract that has many source entries as JSON metadata and ensure they are reflected.
#[tokio::test]
#[serial]
async fn can_fetch_contract_source_tree_for_multi_entry_contract() {
    run_with_client(Chain::mainnet(), |client| async move {
        let meta = client
            .contract_source_code("0x8d04a8c79cEB0889Bdd12acdF3Fa9D207eD3Ff63".parse().unwrap())
            .await
            .unwrap();

        assert_eq!(meta.items.len(), 1);
        assert!(matches!(meta.items[0].source_code, SourceCodeMetadata::Metadata { .. }));
        assert_eq!(meta.source_tree().entries.len(), 15);
    })
    .await
}

/// Query a contract that has a plain source code mapping instead of tagged structures.
#[tokio::test]
#[serial]
async fn can_fetch_contract_source_tree_for_plain_source_code_mapping() {
    run_with_client(Chain::mainnet(), |client| async move {
        let meta = client
            .contract_source_code("0x68b26dcf21180d2a8de5a303f8cc5b14c8d99c4c".parse().unwrap())
            .await
            .unwrap();

        assert_eq!(meta.items.len(), 1);
        assert!(matches!(meta.items[0].source_code, SourceCodeMetadata::Sources(_)));
        assert_eq!(meta.source_tree().entries.len(), 6);
    })
    .await
}

#[tokio::test]
#[serial]
async fn can_fetch_contract_creation_data() {
    run_with_client(Chain::mainnet(), |client| async move {
        client
            .contract_creation_data("0xdac17f958d2ee523a2206206994597c13d831ec7".parse().unwrap())
            .await
            .unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn error_when_creation_data_for_eoa() {
    init_tracing();
    run_with_client(Chain::mainnet(), |client| async move {
        let addr = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045".parse().unwrap();
        let err = client.contract_creation_data(addr).await.unwrap_err();
        assert!(matches!(err, EtherscanError::ContractNotFound(_)));
    })
    .await
}
