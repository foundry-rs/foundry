//! Integration tests for the `anvil_*` namespace served by reth-anvil.

use alloy_network::{TransactionBuilder, TransactionResponse};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::anvil::{Metadata, MineOptions, NodeInfo};
use alloy_rpc_types_eth::{Block, TransactionRequest, state::StateOverridesBuilder};
use eyre::{OptionExt, Result, bail};
use jsonrpsee::{
    core::{ClientError, client::ClientT},
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
};
use reth_anvil::{EthApi, EthereumHardfork, NodeConfig, NodeHandle, spawn};
use serde_json::Value;
use std::{str::FromStr, time::Duration};
use tokio::time::sleep;

async fn with_test_client<F, Fut>(test: F) -> Result<()>
where
    F: FnOnce(HttpClient) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let (_api, _handle, client) = spawn_with_client(NodeConfig::test()).await?;
    test(client).await
}

async fn spawn_with_client(config: NodeConfig) -> Result<(EthApi, NodeHandle, HttpClient)> {
    let (api, handle) = spawn(config).await;
    let client = HttpClientBuilder::default().build(handle.http_endpoint())?;
    Ok((api, handle, client))
}

/// Fetches a receipt.
///
/// Reth recovers the sender from the signature when a transaction misses its RPC cache, which
/// fails for an impersonated transaction until the node reuses cached senders there
/// (<https://github.com/paradigmxyz/reth/pull/27757>). Treat that as "not indexed yet".
async fn get_receipt(client: &HttpClient, tx_hash: B256) -> Result<Option<Value>> {
    match client
        .request::<Option<Value>, _>("eth_getTransactionReceipt", rpc_params![tx_hash])
        .await
    {
        Ok(receipt) => Ok(receipt),
        Err(error) if error.to_string().contains("invalid transaction signature") => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn wait_for_receipt(client: &HttpClient, tx_hash: B256) -> Result<Value> {
    for _ in 0..50 {
        if let Some(receipt) = get_receipt(client, tx_hash).await? {
            return Ok(receipt);
        }
        sleep(Duration::from_millis(100)).await;
    }

    bail!("timed out waiting for receipt for {tx_hash}");
}

async fn assert_no_receipt(client: &HttpClient, tx_hash: B256, attempts: usize) -> Result<()> {
    for _ in 0..attempts {
        if get_receipt(client, tx_hash).await?.is_some() {
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
        let expected_hardfork = EthereumHardfork::from_chain_and_timestamp(
            alloy_chains::Chain::mainnet(),
            expected_timestamp,
        )
        .unwrap_or(EthereumHardfork::Osaka);
        assert_eq!(node_info.hard_fork, expected_hardfork.to_string().to_lowercase());
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

/// Runtime code that returns `BALANCE(target)`.
fn balance_of_code(target: Address) -> Bytes {
    let mut bytecode = Vec::with_capacity(30);
    bytecode.push(0x73);
    bytecode.extend_from_slice(target.as_slice());
    bytecode.extend_from_slice(&[0x31, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    Bytes::from(bytecode)
}

/// Runtime code that returns `SLOAD(0)`.
const SLOAD_ZERO_CODE: Bytes =
    Bytes::from_static(&[0x60, 0x00, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

/// Runtime code that returns 42.
const RETURN_42_CODE: Bytes =
    Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

#[tokio::test]
async fn anvil_set_balance_is_visible_to_reads_and_eth_call() -> Result<()> {
    with_test_client(|client| async move {
        let target = Address::repeat_byte(0xBA);
        let contract = Address::repeat_byte(0xCC);
        let new_balance = U256::from(42_000_000_000_000_000_000u128);

        let before: U256 = client.request("eth_getBalance", rpc_params![target, "latest"]).await?;
        assert_eq!(before, U256::ZERO, "target should start with zero balance");

        client.request::<(), _>("anvil_setBalance", rpc_params![target, new_balance]).await?;

        let after: U256 = client.request("eth_getBalance", rpc_params![target, "latest"]).await?;
        assert_eq!(
            after, new_balance,
            "eth_getBalance should return the value set by anvil_setBalance"
        );

        let state_override =
            StateOverridesBuilder::default().with_code(contract, balance_of_code(target)).build();
        let call = TransactionRequest::default().with_to(contract);
        let result: Bytes =
            client.request("eth_call", rpc_params![call, "latest", state_override]).await?;
        assert_eq!(
            U256::from_be_slice(result.as_ref()),
            new_balance,
            "eth_call should see the balance set by anvil_setBalance"
        );

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_set_balance_funds_transactions_and_lands_in_chain_state() -> Result<()> {
    with_test_client(|client| async move {
        let sender = Address::repeat_byte(0xF1);
        let recipient = Address::repeat_byte(0xF2);
        let funded = U256::from(10_000_000_000_000_000_000u128);
        let value = U256::from(1_000_000_000_000_000_000u128);

        client.request::<(), _>("anvil_setBalance", rpc_params![sender, funded]).await?;
        client.request::<(), _>("anvil_impersonateAccount", rpc_params![sender]).await?;

        let gas_price: u128 =
            client.request::<U256, _>("eth_gasPrice", rpc_params![]).await?.to::<u128>() * 2;
        let tx = TransactionRequest::default()
            .with_from(sender)
            .with_to(recipient)
            .with_gas_price(gas_price)
            .with_gas_limit(21_000)
            .with_value(value);
        let tx_hash: B256 = client.request("eth_sendTransaction", rpc_params![tx]).await?;
        let receipt = wait_for_receipt(&client, tx_hash).await?;
        assert_eq!(receipt["status"].as_str(), Some("0x1"), "transfer should succeed");

        let recipient_balance: U256 =
            client.request("eth_getBalance", rpc_params![recipient, "latest"]).await?;
        assert_eq!(recipient_balance, value);

        let gas_used = U256::from_str(receipt["gasUsed"].as_str().ok_or_eyre("missing gasUsed")?)?;
        let effective_gas_price = U256::from_str(
            receipt["effectiveGasPrice"].as_str().ok_or_eyre("missing effectiveGasPrice")?,
        )?;
        let expected = funded - value - gas_used * effective_gas_price;
        let sender_balance: U256 =
            client.request("eth_getBalance", rpc_params![sender, "latest"]).await?;
        assert_eq!(sender_balance, expected, "the spend should be visible after the block lands");

        let mined_block =
            U256::from_str(receipt["blockNumber"].as_str().ok_or_eyre("missing blockNumber")?)?;
        let historical: U256 = client
            .request("eth_getBalance", rpc_params![sender, format!("0x{mined_block:x}")])
            .await?;
        assert_eq!(
            historical, expected,
            "the chain state at the mined block should hold the balance"
        );

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_add_balance_accumulates_and_is_visible_to_reads() -> Result<()> {
    with_test_client(|client| async move {
        let target = Address::repeat_byte(0xAD);
        let contract = Address::repeat_byte(0xCE);
        let first = U256::from(7u64);
        let second = U256::from(9u64);
        let expected = first + second;

        client.request::<(), _>("anvil_addBalance", rpc_params![target, first]).await?;
        client.request::<(), _>("anvil_addBalance", rpc_params![target, second]).await?;

        let balance: U256 = client.request("eth_getBalance", rpc_params![target, "latest"]).await?;
        assert_eq!(balance, expected, "eth_getBalance should reflect the accumulated balance");

        let state_override =
            StateOverridesBuilder::default().with_code(contract, balance_of_code(target)).build();
        let call = TransactionRequest::default().with_to(contract);
        let result: Bytes =
            client.request("eth_call", rpc_params![call, "latest", state_override]).await?;
        assert_eq!(U256::from_be_slice(result.as_ref()), expected);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_set_nonce_reflected_by_eth_get_transaction_count() -> Result<()> {
    with_test_client(|client| async move {
        let target = Address::repeat_byte(0xAB);
        let new_nonce = U256::from(7u64);

        let before: U256 =
            client.request("eth_getTransactionCount", rpc_params![target, "latest"]).await?;
        assert_eq!(before, U256::ZERO, "target should start with zero nonce");

        client.request::<(), _>("anvil_setNonce", rpc_params![target, new_nonce]).await?;

        let after: U256 =
            client.request("eth_getTransactionCount", rpc_params![target, "latest"]).await?;
        assert_eq!(after, new_nonce, "eth_getTransactionCount should see the override");

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let after_block: U256 =
            client.request("eth_getTransactionCount", rpc_params![target, "latest"]).await?;
        assert_eq!(after_block, new_nonce, "the nonce should persist once a block lands");

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_set_code_is_visible_to_eth_get_code_and_eth_call() -> Result<()> {
    with_test_client(|client| async move {
        let contract = Address::repeat_byte(0xCD);

        let before: Bytes = client.request("eth_getCode", rpc_params![contract, "latest"]).await?;
        assert!(before.is_empty(), "target should start without code");

        client.request::<(), _>("anvil_setCode", rpc_params![contract, RETURN_42_CODE]).await?;

        let after: Bytes = client.request("eth_getCode", rpc_params![contract, "latest"]).await?;
        assert_eq!(after, RETURN_42_CODE, "eth_getCode should see the overridden code");

        let call = TransactionRequest::default().with_to(contract);
        let result: Bytes = client.request("eth_call", rpc_params![call.clone(), "latest"]).await?;
        assert_eq!(U256::from_be_slice(result.as_ref()), U256::from(42u64));

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let after_block: Bytes =
            client.request("eth_getCode", rpc_params![contract, "latest"]).await?;
        assert_eq!(after_block, RETURN_42_CODE, "the code should persist once a block lands");
        let result: Bytes = client.request("eth_call", rpc_params![call, "latest"]).await?;
        assert_eq!(U256::from_be_slice(result.as_ref()), U256::from(42u64));

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_set_storage_at_is_visible_to_eth_get_storage_at_and_eth_call() -> Result<()> {
    with_test_client(|client| async move {
        let contract = Address::repeat_byte(0xCE);
        let slot = U256::ZERO;
        let value = B256::from(U256::from(0xBEEFu64));

        client.request::<(), _>("anvil_setCode", rpc_params![contract, SLOAD_ZERO_CODE]).await?;
        let updated: bool =
            client.request("anvil_setStorageAt", rpc_params![contract, slot, value]).await?;
        assert!(updated, "anvil_setStorageAt should return true");

        let storage: B256 =
            client.request("eth_getStorageAt", rpc_params![contract, slot, "latest"]).await?;
        assert_eq!(storage, value, "eth_getStorageAt should see the overridden storage");

        let call = TransactionRequest::default().with_to(contract);
        let result: Bytes = client.request("eth_call", rpc_params![call.clone(), "latest"]).await?;
        assert_eq!(B256::from_slice(result.as_ref()), value);

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let storage: B256 =
            client.request("eth_getStorageAt", rpc_params![contract, slot, "latest"]).await?;
        assert_eq!(storage, value, "the storage should persist once a block lands");
        let result: Bytes = client.request("eth_call", rpc_params![call, "latest"]).await?;
        assert_eq!(B256::from_slice(result.as_ref()), value);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_snapshot_and_revert_restore_state_and_settings() -> Result<()> {
    with_test_client(|client| async move {
        let account = Address::repeat_byte(0x5A);
        let original_balance = U256::from(123u64);
        let original_gas_limit = U256::from(25_000_000u64);
        let replacement_gas_limit = U256::from(30_000_000u64);

        client.request::<(), _>("anvil_setBalance", rpc_params![account, original_balance]).await?;
        client
            .request::<bool, _>("anvil_setBlockGasLimit", rpc_params![original_gas_limit])
            .await?;
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;

        let snapshot: U256 = client.request("evm_snapshot", rpc_params![]).await?;
        assert_eq!(snapshot, U256::ZERO, "the first snapshot id should be zero");
        let snapshot_block_number = block_number(&client).await?;
        let snapshot_block = get_block(&client, "latest").await?;
        let snapshot_block_hash =
            B256::from_str(snapshot_block["hash"].as_str().ok_or_eyre("missing snapshot hash")?)?;
        let metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
        assert_eq!(
            metadata.snapshots.get(&snapshot),
            Some(&(snapshot_block_number, snapshot_block_hash))
        );

        client.request::<(), _>("anvil_setBalance", rpc_params![account, U256::from(1u64)]).await?;
        client
            .request::<bool, _>("evm_setBlockGasLimit", rpc_params![replacement_gas_limit])
            .await?;
        client.request::<(), _>("anvil_mine", rpc_params![U256::from(3u64)]).await?;
        assert_eq!(block_number(&client).await?, snapshot_block_number + 3);

        let reverted: bool = client.request("evm_revert", rpc_params![snapshot]).await?;
        assert!(reverted, "revert should return true for a known snapshot");
        assert_eq!(block_number(&client).await?, snapshot_block_number);
        let head = get_block(&client, "latest").await?;
        assert_eq!(head["hash"].as_str(), Some(format!("{snapshot_block_hash:#x}").as_str()));

        let balance: U256 =
            client.request("eth_getBalance", rpc_params![account, "latest"]).await?;
        assert_eq!(balance, original_balance, "the revert should restore the snapshot balance");

        let second_revert: bool = client.request("anvil_revert", rpc_params![snapshot]).await?;
        assert!(!second_revert, "snapshot ids are invalid after a revert");
        let metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
        assert!(!metadata.snapshots.contains_key(&snapshot));

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        assert_eq!(block_number(&client).await?, snapshot_block_number + 1);
        let mined = get_block(&client, "latest").await?;
        let gas_limit = U256::from_str(mined["gasLimit"].as_str().ok_or_eyre("missing gasLimit")?)?;
        assert_eq!(
            gas_limit, original_gas_limit,
            "the revert should restore the block env overrides"
        );
        assert_eq!(
            mined["parentHash"].as_str(),
            Some(format!("{snapshot_block_hash:#x}").as_str())
        );

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_revert_drops_mined_transactions() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let recipient = Address::repeat_byte(0x9A);

        let snapshot: U256 = client.request("anvil_snapshot", rpc_params![]).await?;
        let snapshot_block_number = block_number(&client).await?;

        let tx_hash: B256 = client
            .request("eth_sendTransaction", rpc_params![transfer(funder, recipient, gas_price)])
            .await?;
        wait_for_receipt(&client, tx_hash).await?;
        let balance: U256 =
            client.request("eth_getBalance", rpc_params![recipient, "latest"]).await?;
        assert_eq!(balance, U256::from(1));

        let reverted: bool = client.request("anvil_revert", rpc_params![snapshot]).await?;
        assert!(reverted);
        assert_eq!(block_number(&client).await?, snapshot_block_number);

        let balance: U256 =
            client.request("eth_getBalance", rpc_params![recipient, "latest"]).await?;
        assert_eq!(balance, U256::ZERO, "the revert should undo the transfer");
        let receipt: Option<Value> =
            client.request("eth_getTransactionReceipt", rpc_params![tx_hash]).await?;
        let tx: Option<Value> =
            client.request("eth_getTransactionByHash", rpc_params![tx_hash]).await?;
        assert!(tx.is_none(), "the reverted transaction should be unknown");
        assert!(receipt.is_none(), "the reverted transaction should have no receipt");

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        assert_no_receipt(&client, tx_hash, 5).await?;
        let balance: U256 =
            client.request("eth_getBalance", rpc_params![recipient, "latest"]).await?;
        assert_eq!(balance, U256::ZERO, "the reverted transaction must not be mined again");

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_rollback_rewinds_the_given_number_of_blocks() -> Result<()> {
    with_test_client(|client| async move {
        let initial_block = block_number(&client).await?;
        client.request::<(), _>("anvil_mine", rpc_params![U256::from(4u64)]).await?;
        assert_eq!(block_number(&client).await?, initial_block + 4);

        client.request::<(), _>("anvil_rollback", rpc_params![2u64]).await?;
        assert_eq!(block_number(&client).await?, initial_block + 2);

        client.request::<(), _>("anvil_rollback", rpc_params![]).await?;
        assert_eq!(block_number(&client).await?, initial_block + 1);

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        assert_eq!(block_number(&client).await?, initial_block + 2);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn evm_mine_returns_hardhat_result() -> Result<()> {
    with_test_client(|client| async move {
        let initial_block = block_number(&client).await?;
        let result: String = client.request("evm_mine", rpc_params![]).await?;
        assert_eq!(result, "0x0");
        assert_eq!(block_number(&client).await?, initial_block + 1);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn safe_and_finalized_tags_follow_the_epoch_distance() -> Result<()> {
    with_test_client(|client| async move {
        let safe = get_block(&client, "safe").await?;
        let finalized = get_block(&client, "finalized").await?;
        assert_eq!(safe["number"].as_str(), Some("0x0"));
        assert_eq!(finalized["number"].as_str(), Some("0x0"));

        client.request::<(), _>("anvil_mine", rpc_params![U256::from(40u64)]).await?;
        let head = block_number(&client).await?;
        let safe = get_block(&client, "safe").await?;
        let finalized = get_block(&client, "finalized").await?;
        assert_eq!(safe["number"].as_str(), Some(format!("0x{:x}", head - 32).as_str()));
        assert_eq!(finalized["number"].as_str(), Some("0x0"));

        Ok(())
    })
    .await
}

#[tokio::test]
async fn default_config_matches_anvil_defaults() -> Result<()> {
    let (api, handle, client) = spawn_with_client(NodeConfig::test()).await?;

    let chain_id: U256 = client.request("eth_chainId", rpc_params![]).await?;
    assert_eq!(chain_id, U256::from(31337u64));
    assert_eq!(api.chain_id().await?, chain_id);

    let accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let dev_accounts: Vec<Address> = handle.dev_accounts().collect();
    assert_eq!(dev_accounts.len(), 10);
    assert_eq!(&accounts[..10], &dev_accounts[..], "eth_accounts should list the dev accounts");
    for account in &dev_accounts {
        assert_eq!(api.balance(*account, None).await?, handle.genesis_balance());
    }
    assert_eq!(
        handle.genesis_balance(),
        U256::from(100u64) * U256::from(10u64).pow(U256::from(18))
    );

    let genesis = get_block(&client, "0x0").await?;
    assert_eq!(genesis["gasLimit"].as_str(), Some("0x1c9c380"));
    assert_eq!(genesis["baseFeePerGas"].as_str(), Some("0x3b9aca00"));

    let create2_deployer: Bytes =
        api.get_code("0x4e59b44847b379578588920cA78FbF26c0B4956C".parse()?, None).await?;
    assert!(!create2_deployer.is_empty(), "the default create2 deployer should be deployed");

    assert_eq!(api.block_number().await?, U256::ZERO);
    api.mine_one().await?;
    assert_eq!(api.block_number().await?, U256::ONE);

    Ok(())
}

#[tokio::test]
async fn config_overrides_apply_to_genesis() -> Result<()> {
    let config = NodeConfig::test()
        .with_chain_id(Some(1337u64))
        .with_hardfork(Some(EthereumHardfork::Prague.into()))
        .with_gas_limit(Some(50_000_000))
        .with_base_fee(Some(7))
        .with_genesis_timestamp(Some(1_700_000_000u64))
        .with_genesis_balance(U256::from(42u64))
        .with_disable_default_create2_deployer(true);
    let (api, handle, client) = spawn_with_client(config).await?;

    assert_eq!(api.chain_id().await?, U256::from(1337u64));
    let genesis = get_block(&client, "0x0").await?;
    assert_eq!(genesis["gasLimit"].as_str(), Some("0x2faf080"));
    assert_eq!(genesis["baseFeePerGas"].as_str(), Some("0x7"));
    assert_eq!(genesis["timestamp"].as_str(), Some("0x6553f100"));
    let node_info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
    assert_eq!(node_info.hard_fork, "prague");
    let funder = handle.dev_accounts().next().ok_or_eyre("no dev account")?;
    assert_eq!(api.balance(funder, None).await?, U256::from(42u64));
    let create2_deployer: Bytes =
        api.get_code("0x4e59b44847b379578588920cA78FbF26c0B4956C".parse()?, None).await?;
    assert!(create2_deployer.is_empty(), "the create2 deployer should be disabled");

    Ok(())
}

#[tokio::test]
async fn no_mining_and_block_time_set_the_initial_mining_mode() -> Result<()> {
    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_no_mining(true)).await?;
    let automine: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    assert!(!automine, "no_mining should disable automine");

    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_blocktime(Some(Duration::from_secs(1)))).await?;
    let automine: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    let interval: Option<u64> = client.request("anvil_getIntervalMining", rpc_params![]).await?;
    assert!(!automine);
    assert_eq!(interval, Some(1));
    wait_for_block_number(&client, 1).await?;

    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_mixed_mining(true, Some(Duration::from_secs(1))))
            .await?;
    let automine: bool = client.request("anvil_getAutomine", rpc_params![]).await?;
    let interval: Option<u64> = client.request("anvil_getIntervalMining", rpc_params![]).await?;
    assert!(automine, "mixed mining should report automine");
    assert_eq!(interval, Some(1), "mixed mining should report the interval");

    Ok(())
}

#[tokio::test]
async fn auto_impersonate_config_signs_for_any_account() -> Result<()> {
    let (api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_auto_impersonate(true)).await?;
    let sender = Address::repeat_byte(0xA7);
    api.anvil_set_balance(sender, U256::from(10u64).pow(U256::from(18))).await?;
    let (_, gas_price) = funder_and_gas_price(&client).await?;
    let tx_hash =
        api.send_transaction(transfer(sender, Address::repeat_byte(0xA8), gas_price)).await?;
    let receipt = api.transaction_receipt(tx_hash).await?;
    let receipt = match receipt {
        Some(receipt) => receipt,
        None => {
            wait_for_receipt(&client, tx_hash).await?;
            api.transaction_receipt(tx_hash).await?.ok_or_eyre("missing receipt")?
        }
    };
    assert!(receipt.status(), "the impersonated transfer should succeed");
    assert_eq!(api.balance(Address::repeat_byte(0xA8), None).await?, U256::from(1));
    Ok(())
}

#[tokio::test]
async fn anvil_reorg_rewinds_and_mines_the_given_transactions() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let recipient = Address::repeat_byte(0x7E);
        client.request::<(), _>("anvil_mine", rpc_params![U256::from(3u64)]).await?;
        let height = block_number(&client).await?;

        let tx = transfer(funder, recipient, gas_price);
        client
            .request::<(), _>(
                "anvil_reorg",
                rpc_params![serde_json::json!({ "depth": 2, "tx_block_pairs": [[tx, 1]] })],
            )
            .await?;

        assert_eq!(block_number(&client).await?, height, "the reorg keeps the chain height");
        let first = get_block(&client, format!("0x{:x}", height - 1)).await?;
        assert_eq!(first["transactions"].as_array().map(Vec::len), Some(0));
        let second = get_block(&client, format!("0x{height:x}")).await?;
        assert_eq!(second["transactions"].as_array().map(Vec::len), Some(1));
        let balance: U256 =
            client.request("eth_getBalance", rpc_params![recipient, "latest"]).await?;
        assert_eq!(balance, U256::from(1));

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_reset_returns_to_genesis() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let recipient = Address::repeat_byte(0x5E);
        let tx_hash: B256 = client
            .request("eth_sendTransaction", rpc_params![transfer(funder, recipient, gas_price)])
            .await?;
        wait_for_receipt(&client, tx_hash).await?;
        client.request::<(), _>("anvil_setBalance", rpc_params![recipient, U256::from(5)]).await?;
        let before: Metadata = client.request("anvil_metadata", rpc_params![]).await?;

        client.request::<(), _>("anvil_reset", rpc_params![]).await?;

        assert_eq!(block_number(&client).await?, 0);
        let balance: U256 =
            client.request("eth_getBalance", rpc_params![recipient, "latest"]).await?;
        assert_eq!(balance, U256::ZERO, "reset should drop state writes and mined transfers");
        let after: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
        assert_ne!(before.instance_id, after.instance_id, "reset should pick a new instance id");

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        assert_eq!(block_number(&client).await?, 1);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn anvil_deal_erc20_and_allowance_override_token_storage() -> Result<()> {
    with_test_client(|client| async move {
        let token = Address::repeat_byte(0xD0);
        let holder = Address::repeat_byte(0xD1);
        let spender = Address::repeat_byte(0xD4);
        let balance = U256::from(500u64);
        let allowance = U256::from(777u64);

        client.request::<(), _>("anvil_setCode", rpc_params![token, SLOAD_ZERO_CODE]).await?;

        client.request::<(), _>("anvil_dealERC20", rpc_params![holder, token, balance]).await?;
        let mut calldata = vec![0x70, 0xa0, 0x82, 0x31];
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(holder.as_slice());
        let call = TransactionRequest::default().with_to(token).with_input(Bytes::from(calldata));
        let result: Bytes = client.request("eth_call", rpc_params![call, "latest"]).await?;
        assert_eq!(U256::from_be_slice(result.as_ref()), balance);

        client
            .request::<(), _>(
                "anvil_setERC20Allowance",
                rpc_params![holder, spender, token, allowance],
            )
            .await?;
        let mut calldata = vec![0xdd, 0x62, 0xed, 0x3e];
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(holder.as_slice());
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(spender.as_slice());
        let call = TransactionRequest::default().with_to(token).with_input(Bytes::from(calldata));
        let result: Bytes = client.request("eth_call", rpc_params![call, "latest"]).await?;
        assert_eq!(U256::from_be_slice(result.as_ref()), allowance);

        Ok(())
    })
    .await
}

#[tokio::test]
async fn misc_anvil_methods_match_anvil() -> Result<()> {
    with_test_client(|client| async move {
        let err = client
            .request::<(), _>("anvil_setMinGasPrice", rpc_params![U256::from(1)])
            .await
            .expect_err("setMinGasPrice is rejected after London");
        assert!(err.to_string().contains("EIP-1559"), "{err}");

        client.request::<(), _>("anvil_setLoggingEnabled", rpc_params![false]).await?;

        let before: u64 = client.request("anvil_getLastBlockWallTime", rpc_params![]).await?;
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let after: u64 = client.request("anvil_getLastBlockWallTime", rpc_params![]).await?;
        assert!(after >= before);

        let blobs: Option<Vec<alloy_consensus::Blob>> =
            client.request("anvil_getBlobsByTransactionHash", rpc_params![B256::ZERO]).await?;
        assert!(blobs.is_none());

        Ok(())
    })
    .await
}
