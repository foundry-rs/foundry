use crate::run_with_client;
use alloy_chains::Chain;
use foundry_block_explorers::block_number::BlockNumber;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn check_get_block_by_timestamp_before() {
    run_with_client(Chain::mainnet(), |client| async move {
        let block_no = client.get_block_by_timestamp(1577836800, "before").await;
        assert!(block_no.is_ok());

        let block_no = block_no.unwrap().block_number;
        assert_eq!(block_no, "9193265".parse::<BlockNumber>().unwrap());
    })
    .await
}

#[tokio::test]
#[serial]
async fn check_get_block_by_timestamp_after() {
    run_with_client(Chain::mainnet(), |client| async move {
        let block_no = client.get_block_by_timestamp(1577836800, "after").await;

        let block_no = block_no.unwrap().block_number;
        assert_eq!(block_no, "9193266".parse::<BlockNumber>().unwrap());
    })
    .await
}
