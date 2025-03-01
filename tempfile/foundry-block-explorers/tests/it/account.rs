use crate::run_with_client;
use alloy_chains::{Chain, NamedChain};
use alloy_primitives::{U256, U64};
use foundry_block_explorers::{
    account::{InternalTxQueryOption, TokenQueryOption},
    block_number::BlockNumber,
};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn get_ether_balance_single_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let balance = client
            .get_ether_balance_single(
                &"0x58eB28A67731c570Ef827C365c89B5751F9E6b0a".parse().unwrap(),
                None,
            )
            .await;
        balance.unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_ether_balance_multi_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let balances = client
            .get_ether_balance_multi(
                &["0x58eB28A67731c570Ef827C365c89B5751F9E6b0a".parse().unwrap()],
                None,
            )
            .await;
        assert!(balances.is_ok());
        let balances = balances.unwrap();
        assert_eq!(balances.len(), 1);
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_transactions_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let txs = client
            .get_transactions(&"0x4F26FfBe5F04ED43630fdC30A87638d53D0b0876".parse().unwrap(), None)
            .await;
        txs.unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_internal_transactions_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let txs = client
            .get_internal_transactions(
                InternalTxQueryOption::ByAddress(
                    "0x2c1ba59d6f58433fb1eaee7d20b26ed83bda51a3".parse().unwrap(),
                ),
                None,
            )
            .await;
        txs.unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_internal_transactions_by_tx_hash_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let txs = client
            .get_internal_transactions(
                InternalTxQueryOption::ByTransactionHash(
                    "0x40eb908387324f2b575b4879cd9d7188f69c8fc9d87c901b9e2daaea4b442170"
                        .parse()
                        .unwrap(),
                ),
                None,
            )
            .await;
        txs.unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_erc20_transfer_events_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let txs = client
            .get_erc20_token_transfer_events(
                TokenQueryOption::ByAddress(
                    "0x4e83362442b8d1bec281594cea3050c8eb01311c".parse().unwrap(),
                ),
                None,
            )
            .await
            .unwrap();
        let tx = txs.first().unwrap();
        assert_eq!(tx.gas_used, U256::from(93657u64));
        assert_eq!(tx.nonce, U256::from(10u64));
        assert_eq!(tx.block_number, BlockNumber::Number(U64::from(2228258u64)));
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_erc721_transfer_events_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let txs = client
            .get_erc721_token_transfer_events(
                TokenQueryOption::ByAddressAndContract(
                    "0x6975be450864c02b4613023c2152ee0743572325".parse().unwrap(),
                    "0x06012c8cf97bead5deae237070f9587f8e7a266d".parse().unwrap(),
                ),
                None,
            )
            .await;
        txs.unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_erc1155_transfer_events_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        let txs = client
            .get_erc1155_token_transfer_events(
                TokenQueryOption::ByAddressAndContract(
                    "0x216CD350a4044e7016f14936663e2880Dd2A39d7".parse().unwrap(),
                    "0x495f947276749ce646f68ac8c248420045cb7b5e".parse().unwrap(),
                ),
                None,
            )
            .await;
        txs.unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_mined_blocks_success() {
    run_with_client(Chain::mainnet(), |client| async move {
        client
            .get_mined_blocks(
                &"0x9dd134d14d1e65f84b706d6f205cd5b1cd03a46b".parse().unwrap(),
                None,
                None,
            )
            .await
            .unwrap();
    })
    .await
}

#[tokio::test]
#[serial]
async fn get_avalanche_transactions() {
    run_with_client(Chain::from_named(NamedChain::Avalanche), |client| async move {
        let txs = client
            .get_transactions(&"0x1549ea9b546ba9ffb306d78a1e1f304760cc4abf".parse().unwrap(), None)
            .await;
        txs.unwrap();
    })
    .await
}
