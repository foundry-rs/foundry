//! Integration tests for the `anvil_*` namespace served by reth-anvil.

use alloy_network::{TransactionBuilder, TransactionResponse};
use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::anvil::{Metadata, MineOptions, NodeInfo};
use alloy_rpc_types_eth::{Block, TransactionRequest};
use eyre::{OptionExt, Result, bail};
use jsonrpsee::{
    core::{ClientError, client::ClientT},
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
};
use reth_anvil::{RethAnvilConfig, launch};
use reth_ethereum::{
    node::core::args::RpcServerArgs,
    tasks::{RuntimeBuilder, RuntimeConfig},
};
use serde_json::Value;
use std::{str::FromStr, time::Duration};
use tokio::time::sleep;

async fn with_test_client<F, Fut>(test: F) -> Result<()>
where
    F: FnOnce(HttpClient) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let runtime = RuntimeBuilder::new(RuntimeConfig::default()).build()?;
    let config = RethAnvilConfig {
        rpc: RpcServerArgs::default().with_unused_ports().with_http(),
        ..Default::default()
    };
    let node = launch(config, runtime).await?;
    let addr =
        node.rpc_server_handles.rpc.http_local_addr().ok_or_eyre("http server did not start")?;
    let client = HttpClientBuilder::default().build(format!("http://{addr}"))?;
    test(client).await
}

async fn wait_for_receipt(client: &HttpClient, tx_hash: B256) -> Result<Value> {
    for _ in 0..50 {
        let receipt = client
            .request::<Option<Value>, _>("eth_getTransactionReceipt", rpc_params![tx_hash])
            .await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        sleep(Duration::from_millis(100)).await;
    }

    bail!("timed out waiting for receipt for {tx_hash}");
}

async fn assert_no_receipt(client: &HttpClient, tx_hash: B256, attempts: usize) -> Result<()> {
    for _ in 0..attempts {
        let receipt = client
            .request::<Option<Value>, _>("eth_getTransactionReceipt", rpc_params![tx_hash])
            .await?;
        if receipt.is_some() {
            bail!("unexpected receipt for {tx_hash}");
        }
        sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}

async fn block_number(client: &HttpClient) -> Result<u64> {
    Ok(client.request::<U256, _>("eth_blockNumber", rpc_params![]).await?.to::<u64>())
}

async fn wait_for_block_number(client: &HttpClient, expected: u64) -> Result<()> {
    let mut last_seen = 0;

    for _ in 0..200 {
        let current = block_number(client).await?;
        if current >= expected {
            return Ok(());
        }

        last_seen = current;
        sleep(Duration::from_millis(100)).await;
    }

    bail!("timed out waiting for block {expected}, last seen {last_seen}");
}

async fn get_block(client: &HttpClient, tag: impl Into<Value>) -> Result<Value> {
    Ok(client.request("eth_getBlockByNumber", rpc_params![tag.into(), false]).await?)
}

async fn block_timestamp(client: &HttpClient, tag: impl Into<Value>) -> Result<u64> {
    let block = get_block(client, tag).await?;
    Ok(U256::from_str(block["timestamp"].as_str().ok_or_eyre("missing block timestamp")?)?
        .to::<u64>())
}

/// Returns the first dev account and a gas price above the current one.
async fn funder_and_gas_price(client: &HttpClient) -> Result<(Address, u128)> {
    let dev_accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let funder = *dev_accounts.first().ok_or_eyre("no dev account available")?;
    let gas_price = client.request::<U256, _>("eth_gasPrice", rpc_params![]).await?.to::<u128>()
        + 1_000_000_000u128;
    Ok((funder, gas_price))
}

fn transfer(from: Address, to: Address, gas_price: u128) -> TransactionRequest {
    TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_gas_price(gas_price)
        .with_value(U256::from(1))
}

#[tokio::test]
async fn explicit_impersonation_allows_eth_send_transaction() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let target = Address::repeat_byte(0x11);
        let recipient = Address::repeat_byte(0x22);

        let funding_tx = TransactionRequest::default()
            .with_from(funder)
            .with_to(target)
            .with_gas_price(gas_price)
            .with_value(U256::from(1_000_000_000_000_000_000u64));
        let funding_hash: B256 =
            client.request("eth_sendTransaction", rpc_params![funding_tx]).await?;
        wait_for_receipt(&client, funding_hash).await?;

        client.request::<(), _>("hardhat_impersonateAccount", rpc_params![target]).await?;

        let impersonated_hash: B256 = client
            .request("eth_sendTransaction", rpc_params![transfer(target, recipient, gas_price)])
            .await?;
        wait_for_receipt(&client, impersonated_hash).await?;

        client.request::<(), _>("hardhat_stopImpersonatingAccount", rpc_params![target]).await?;

        let err: ClientError = client
            .request::<B256, _>(
                "eth_sendTransaction",
                rpc_params![transfer(target, recipient, gas_price)],
            )
            .await
            .expect_err("stopped impersonation should reject eth_sendTransaction");
        assert!(
            err.to_string().contains("unknown account"),
            "unexpected error after stop impersonating: {err}"
        );

        Ok(())
    })
    .await
}

#[tokio::test]
async fn set_automine_controls_transaction_mining() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let initial_block = block_number(&client).await?;
        let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
        assert!(enabled, "automine should be enabled by default");
        let interval: Option<u64> =
            client.request("anvil_getIntervalMining", rpc_params![]).await?;
        assert_eq!(interval, None, "interval mining should be unset by default");

        client.request::<(), _>("evm_setAutomine", rpc_params![false]).await?;
        let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
        assert!(!enabled, "automine should be disabled after evm_setAutomine(false)");

        let tx_hash: B256 = client
            .request(
                "eth_sendTransaction",
                rpc_params![transfer(funder, Address::repeat_byte(0x33), gas_price)],
            )
            .await?;
        assert_no_receipt(&client, tx_hash, 10).await?;
        assert_eq!(block_number(&client).await?, initial_block);

        client.request::<(), _>("anvil_setAutomine", rpc_params![true]).await?;
        wait_for_receipt(&client, tx_hash).await?;
        assert_eq!(block_number(&client).await?, initial_block + 1);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_mine_advances_requested_blocks() -> Result<()> {
    with_test_client(|client| async move {
        let initial_block = block_number(&client).await?;

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        wait_for_block_number(&client, initial_block + 1).await?;

        client.request::<(), _>("hardhat_mine", rpc_params![U256::from(2)]).await?;
        wait_for_block_number(&client, initial_block + 3).await?;
        let pre_interval_timestamp = block_timestamp(&client, "latest").await?;

        client.request::<(), _>("anvil_mine", rpc_params![U256::from(2), U256::from(10)]).await?;
        wait_for_block_number(&client, initial_block + 5).await?;

        let first_interval_ts =
            block_timestamp(&client, format!("0x{:x}", initial_block + 4)).await?;
        let second_interval_ts =
            block_timestamp(&client, format!("0x{:x}", initial_block + 5)).await?;
        assert_eq!(first_interval_ts, pre_interval_timestamp + 10);
        assert_eq!(second_interval_ts, first_interval_ts + 10);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn set_interval_mining_controls_block_production() -> Result<()> {
    with_test_client(|client| async move {
        let initial_block = block_number(&client).await?;

        client.request::<(), _>("evm_setIntervalMining", rpc_params![2u64]).await?;
        let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
        assert!(!enabled, "automine should be false in interval mining mode");
        let interval: Option<u64> =
            client.request("anvil_getIntervalMining", rpc_params![]).await?;
        assert_eq!(interval, Some(2), "interval mining should report the configured value");

        client.request::<(), _>("evm_setAutomine", rpc_params![false]).await?;
        let interval: Option<u64> =
            client.request("anvil_getIntervalMining", rpc_params![]).await?;
        assert_eq!(interval, Some(2), "disabling automine should not clear interval mining");

        wait_for_block_number(&client, initial_block + 1).await?;

        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let tx_hash: B256 = client
            .request(
                "eth_sendTransaction",
                rpc_params![transfer(funder, Address::repeat_byte(0x44), gas_price)],
            )
            .await?;
        wait_for_receipt(&client, tx_hash).await?;

        client.request::<(), _>("anvil_setIntervalMining", rpc_params![0u64]).await?;
        let enabled: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
        assert!(!enabled, "manual mode should not report automine");
        let interval: Option<u64> =
            client.request("anvil_getIntervalMining", rpc_params![]).await?;
        assert_eq!(interval, None, "zero interval should disable interval mining");
        let tx_hash: B256 = client
            .request(
                "eth_sendTransaction",
                rpc_params![transfer(funder, Address::repeat_byte(0x55), gas_price)],
            )
            .await?;
        assert_no_receipt(&client, tx_hash, 10).await?;
        let block_after_manual = block_number(&client).await?;
        sleep(Duration::from_millis(1200)).await;
        assert_eq!(
            block_number(&client).await?,
            block_after_manual,
            "manual mode should not keep producing interval blocks",
        );

        let tx_hash: B256 = client
            .request(
                "eth_sendTransaction",
                rpc_params![transfer(funder, Address::repeat_byte(0x66), gas_price)],
            )
            .await?;
        assert_no_receipt(&client, tx_hash, 5).await?;

        client.request::<(), _>("anvil_mine", rpc_params![U256::from(1), U256::ZERO]).await?;
        wait_for_receipt(&client, tx_hash).await?;

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_mine_detailed_returns_full_blocks() -> Result<()> {
    with_test_client(|client| async move {
        client.request::<(), _>("evm_setAutomine", rpc_params![false]).await?;

        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let tx_hash: B256 = client
            .request(
                "eth_sendTransaction",
                rpc_params![transfer(funder, Address::repeat_byte(0x77), gas_price)],
            )
            .await?;

        let initial_block = block_number(&client).await?;
        let blocks: Vec<Block> = client
            .request(
                "anvil_mine_detailed",
                rpc_params![MineOptions::Options { timestamp: None, blocks: Some(2) }],
            )
            .await?;

        assert_eq!(blocks.len(), 2, "should return the requested number of blocks");
        assert_eq!(blocks[0].number(), initial_block + 1);
        assert_eq!(blocks[1].number(), initial_block + 2);

        let first_block_txs = blocks[0]
            .transactions
            .as_transactions()
            .ok_or_eyre("anvil_mine_detailed should return full transactions")?;
        assert_eq!(first_block_txs.len(), 1, "pending tx should be mined into first block");
        assert_eq!(first_block_txs[0].tx_hash(), tx_hash);

        let second_block_txs = blocks[1]
            .transactions
            .as_transactions()
            .ok_or_eyre("anvil_mine_detailed should return full transactions")?;
        assert!(
            second_block_txs.is_empty(),
            "second block should be empty when there are no pending txs",
        );

        wait_for_receipt(&client, tx_hash).await?;

        let latest_timestamp = block_timestamp(&client, "latest").await?;
        let requested_timestamp = latest_timestamp + 15;
        let blocks: Vec<Block> = client
            .request(
                "evm_mine_detailed",
                rpc_params![MineOptions::Timestamp(Some(requested_timestamp))],
            )
            .await?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].header.timestamp, requested_timestamp);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_set_and_remove_block_timestamp_interval() -> Result<()> {
    with_test_client(|client| async move {
        let removed: bool =
            client.request("anvil_removeBlockTimestampInterval", rpc_params![]).await?;
        assert!(!removed, "should return false when no interval is set");

        client.request::<(), _>("anvil_setBlockTimestampInterval", rpc_params![10u64]).await?;

        let removed: bool =
            client.request("anvil_removeBlockTimestampInterval", rpc_params![]).await?;
        assert!(removed, "should return true when an interval was removed");

        let removed: bool =
            client.request("anvil_removeBlockTimestampInterval", rpc_params![]).await?;
        assert!(!removed, "should return false after interval was already removed");

        client.request::<(), _>("anvil_setBlockTimestampInterval", rpc_params![10u64]).await?;
        client.request::<(), _>("anvil_setBlockTimestampInterval", rpc_params![20u64]).await?;

        let removed: bool =
            client.request("anvil_removeBlockTimestampInterval", rpc_params![]).await?;
        assert!(removed, "should return true after overwritten interval");

        let removed: bool =
            client.request("anvil_removeBlockTimestampInterval", rpc_params![]).await?;
        assert!(!removed, "the second set should overwrite the interval, not stack");

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_time_controls_are_visible_in_mined_blocks() -> Result<()> {
    with_test_client(|client| async move {
        let latest_timestamp = block_timestamp(&client, "latest").await?;

        let _increased: i64 =
            client.request("anvil_increaseTime", rpc_params![U256::from(30u64)]).await?;

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let after_increase = block_timestamp(&client, "latest").await?;
        assert!(
            after_increase >= latest_timestamp + 30,
            "increaseTime should move the next mined block forward by at least the requested amount",
        );

        let exact_timestamp = after_increase + 25;
        client
            .request::<(), _>("anvil_setNextBlockTimestamp", rpc_params![exact_timestamp])
            .await?;
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let after_exact = block_timestamp(&client, "latest").await?;
        assert_eq!(after_exact, exact_timestamp);

        let reset_timestamp = exact_timestamp + 40;
        let offset: u64 = client.request("anvil_setTime", rpc_params![reset_timestamp]).await?;
        assert!(offset <= 40, "setTime offset should not exceed requested jump");
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let after_reset = block_timestamp(&client, "latest").await?;
        assert!(
            after_reset >= reset_timestamp,
            "setTime should move the time baseline forward without pinning an exact next-block timestamp",
        );

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_node_info_and_metadata_follow_latest_head() -> Result<()> {
    with_test_client(|client| async move {
        let expected_block_number = block_number(&client).await?;
        let expected_gas_price: u128 =
            client.request::<U256, _>("eth_gasPrice", rpc_params![]).await?.to::<u128>();
        let latest_block = get_block(&client, "latest").await?;
        let expected_hash =
            B256::from_str(latest_block["hash"].as_str().ok_or_eyre("missing latest hash")?)?;
        let expected_timestamp = U256::from_str(
            latest_block["timestamp"].as_str().ok_or_eyre("missing latest timestamp")?,
        )?
        .to::<u64>();

        let node_info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
        let metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
        let hardhat_metadata: Metadata = client.request("hardhat_metadata", rpc_params![]).await?;

        assert_eq!(node_info.current_block_number, expected_block_number);
        assert_eq!(node_info.current_block_timestamp, expected_timestamp);
        assert_eq!(node_info.current_block_hash, expected_hash);
        assert_eq!(node_info.hard_fork, "osaka");
        assert_eq!(node_info.transaction_order, "fees");
        assert_eq!(node_info.environment.chain_id, metadata.chain_id);
        assert_eq!(node_info.environment.gas_price, expected_gas_price);
        assert_eq!(metadata.latest_block_number, expected_block_number);
        assert_eq!(metadata.latest_block_hash, expected_hash);
        assert_eq!(metadata.client_version, format!("reth-anvil/v{}", env!("CARGO_PKG_VERSION")));
        assert_eq!(metadata.client_semver.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert!(metadata.snapshots.is_empty());
        assert_eq!(hardhat_metadata, metadata);

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;

        let mined_number = block_number(&client).await?;
        let mined_info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
        let mined_metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;

        assert_eq!(mined_info.current_block_number, mined_number);
        assert_eq!(mined_metadata.latest_block_number, mined_number);
        assert_eq!(mined_info.current_block_hash, mined_metadata.latest_block_hash);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn set_block_gas_limit_accepts_anvil_and_evm_namespaces() -> Result<()> {
    with_test_client(|client| async move {
        for (method, custom_limit) in [
            ("evm_setBlockGasLimit", U256::from(20_000_000u64)),
            ("anvil_setBlockGasLimit", U256::from(21_000_000u64)),
        ] {
            let ok: bool = client.request(method, rpc_params![custom_limit]).await?;
            assert!(ok, "{method} should return true");

            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let block1 = get_block(&client, "latest").await?;
            let gas_limit_1 =
                U256::from_str(block1["gasLimit"].as_str().ok_or_eyre("missing gasLimit")?)?;
            assert_eq!(gas_limit_1, custom_limit, "{method} should affect the first mined block");

            client.request::<(), _>("anvil_mine", rpc_params![]).await?;
            let block2 = get_block(&client, "latest").await?;
            let gas_limit_2 =
                U256::from_str(block2["gasLimit"].as_str().ok_or_eyre("missing gasLimit")?)?;
            assert_eq!(
                gas_limit_2, custom_limit,
                "{method} gas limit should persist across blocks"
            );
        }

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_set_coinbase_persists_across_blocks() -> Result<()> {
    with_test_client(|client| async move {
        let coinbase = Address::repeat_byte(0xCB);

        client.request::<(), _>("anvil_setCoinbase", rpc_params![coinbase]).await?;

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let block1 = get_block(&client, "latest").await?;
        let miner_1 = Address::from_str(block1["miner"].as_str().ok_or_eyre("missing miner")?)?;
        assert_eq!(miner_1, coinbase, "first mined block should use the overridden coinbase");

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let block2 = get_block(&client, "latest").await?;
        let miner_2 = Address::from_str(block2["miner"].as_str().ok_or_eyre("missing miner")?)?;
        assert_eq!(miner_2, coinbase, "coinbase should persist across blocks");

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_set_next_block_base_fee_per_gas_is_consumed_once() -> Result<()> {
    with_test_client(|client| async move {
        let custom_base_fee = U256::from(42_000_000_000u64);

        client
            .request::<(), _>("anvil_setNextBlockBaseFeePerGas", rpc_params![custom_base_fee])
            .await?;

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let target_block = get_block(&client, "latest").await?;
        let base_fee = U256::from_str(
            target_block["baseFeePerGas"].as_str().ok_or_eyre("missing baseFeePerGas")?,
        )?;
        assert_eq!(
            base_fee, custom_base_fee,
            "next mined block should use the overridden base fee"
        );

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let after_block = get_block(&client, "latest").await?;
        let after_base_fee = U256::from_str(
            after_block["baseFeePerGas"].as_str().ok_or_eyre("missing baseFeePerGas")?,
        )?;
        assert_ne!(
            after_base_fee, custom_base_fee,
            "base fee override should be consumed after one block"
        );

        Ok(())
    })
    .await
}
