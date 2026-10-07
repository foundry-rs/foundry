//! Integration tests for the `anvil_*` namespace served by reth-anvil.

use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::Encodable2718;
use alloy_network::{TransactionBuilder, TransactionResponse, TxSignerSync};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types::anvil::{Forking, Metadata, MineOptions, NodeInfo};
use alloy_rpc_types_eth::{Block, TransactionRequest, state::StateOverridesBuilder};
use alloy_signer_local::PrivateKeySigner;
use eyre::{OptionExt, Result, bail};
use jsonrpsee::{
    core::{
        ClientError,
        client::{ClientT, SubscriptionClientT},
    },
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
    ws_client::WsClientBuilder,
};
use reth_anvil::{EthApi, EthereumHardfork, NodeConfig, NodeHandle, spawn};
use serde_json::Value;
use std::{str::FromStr, time::Duration};
use tokio::time::sleep;

#[cfg(feature = "monad")]
mod monad;

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
        // The dev chain id is not a known chain, so the latest hardfork is active, as in anvil.
        let expected_hardfork = EthereumHardfork::default();
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
    let tx_hash = api
        .send_transaction(transfer(sender, Address::repeat_byte(0xA8), gas_price).into())
        .await?;
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
    let (api, _handle, client) = spawn_with_client(NodeConfig::test()).await?;
    {
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
        assert_eq!(api.instance_id(), after.instance_id, "the in-process api shares the id");

        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        assert_eq!(block_number(&client).await?, 1);

        Ok(())
    }
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

/// Mainnet block the fork tests fork from, and known state at that block.
const FORK_BLOCK_NUMBER: u64 = 14_608_400;
const FORK_BLOCK_TIMESTAMP: u64 = 1_650_274_250;
const DEAD_BALANCE_AT_FORK_BLOCK: u128 = 12_556_069_338_441_120_059_867;
const WETH: Address = alloy_primitives::address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");

fn fork_config() -> NodeConfig {
    NodeConfig::test()
        .with_eth_rpc_url(Some(foundry_test_utils::rpc::next_http_archive_rpc_url()))
        .with_fork_block_number(Some(FORK_BLOCK_NUMBER))
}

async fn balance(client: &HttpClient, address: Address, tag: impl Into<Value>) -> Result<U256> {
    Ok(client.request("eth_getBalance", rpc_params![address, tag.into()]).await?)
}

#[tokio::test]
async fn fork_serves_remote_state_and_blocks() -> Result<()> {
    let (_api, handle, client) = spawn_with_client(fork_config()).await?;

    assert_eq!(block_number(&client).await?, FORK_BLOCK_NUMBER);
    let chain_id: U256 = client.request("eth_chainId", rpc_params![]).await?;
    assert_eq!(chain_id, U256::from(1));
    assert_eq!(block_timestamp(&client, "latest").await?, FORK_BLOCK_TIMESTAMP);

    // Remote state at the fork block.
    assert_eq!(
        balance(&client, Address::with_last_byte(0xad).create(0), "latest").await?,
        U256::ZERO
    );
    let dead_address = Address::from_str("0x000000000000000000000000000000000000dEaD")?;
    assert_eq!(
        balance(&client, dead_address, "latest").await?,
        U256::from(DEAD_BALANCE_AT_FORK_BLOCK)
    );
    let code: Bytes = client.request("eth_getCode", rpc_params![WETH, "latest"]).await?;
    assert!(!code.is_empty(), "the fork serves remote code");

    // The dev accounts are funded on top of the remote state.
    let funder = handle.dev_accounts().next().ok_or_eyre("no dev account")?;
    assert_eq!(balance(&client, funder, "latest").await?, handle.genesis_balance());

    // Remote blocks below the fork block, linked to the fork block.
    let fork_block = get_block(&client, format!("0x{FORK_BLOCK_NUMBER:x}")).await?;
    let parent = get_block(&client, format!("0x{:x}", FORK_BLOCK_NUMBER - 1)).await?;
    assert_eq!(fork_block["parentHash"], parent["hash"]);
    assert_eq!(parent["number"], Value::String(format!("0x{:x}", FORK_BLOCK_NUMBER - 1)));
    let by_hash: Value =
        client.request("eth_getBlockByHash", rpc_params![parent["hash"].clone(), true]).await?;
    assert_eq!(by_hash["number"], parent["number"]);
    assert!(!by_hash["transactions"].as_array().ok_or_eyre("transactions")?.is_empty());

    // Remote receipts and transactions.
    let tx_hash = by_hash["transactions"][0]["hash"].clone();
    let receipt: Value =
        client.request("eth_getTransactionReceipt", rpc_params![tx_hash.clone()]).await?;
    assert_eq!(receipt["blockHash"], parent["hash"]);
    let tx: Value = client.request("eth_getTransactionByHash", rpc_params![tx_hash]).await?;
    assert_eq!(tx["blockNumber"], parent["number"]);

    // Historical remote state.
    let earlier = balance(&client, WETH, format!("0x{:x}", FORK_BLOCK_NUMBER - 1000)).await?;
    assert!(!earlier.is_zero(), "WETH holds ETH at every block");

    // A call against a remote contract.
    let call = TransactionRequest::default()
        .with_to(WETH)
        .with_input(Bytes::from_static(&[0x95, 0xd8, 0x9b, 0x41]));
    let symbol: Bytes = client.request("eth_call", rpc_params![call, "latest"]).await?;
    assert!(symbol.windows(4).any(|window| window == b"WETH"), "symbol() returns WETH");

    let metadata: Metadata = client.request("anvil_metadata", rpc_params![]).await?;
    let forked = metadata.forked_network.ok_or_eyre("forked network")?;
    assert_eq!(forked.chain_id, 1);
    assert_eq!(forked.fork_block_number, FORK_BLOCK_NUMBER);
    assert_eq!(forked.fork_block_hash.to_string(), fork_block["hash"]);
    let info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
    assert_eq!(info.fork_config.fork_block_number, Some(FORK_BLOCK_NUMBER));
    assert!(info.fork_config.fork_url.is_some());

    Ok(())
}

#[tokio::test]
async fn fork_mines_and_reverts_on_top_of_remote_state() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(fork_config()).await?;
    let dead = Address::from_str("0x000000000000000000000000000000000000dEaD")?;
    let remote_balance = U256::from(DEAD_BALANCE_AT_FORK_BLOCK);
    let (funder, gas_price) = funder_and_gas_price(&client).await?;
    // The dev accounts have a history on the forked chain.
    let remote_nonce: U256 =
        client.request("eth_getTransactionCount", rpc_params![funder, "latest"]).await?;

    let snapshot: U256 = client.request("evm_snapshot", rpc_params![]).await?;

    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![transfer(funder, dead, gas_price)])
        .await?;
    let receipt = wait_for_receipt(&client, tx_hash).await?;
    assert_eq!(receipt["status"], "0x1");
    assert_eq!(block_number(&client).await?, FORK_BLOCK_NUMBER + 1);

    // The local write wins over the remote state, and the remote value stays at the fork block.
    let sent = U256::from(1);
    assert_eq!(balance(&client, dead, "latest").await?, remote_balance + sent);
    assert_eq!(balance(&client, dead, format!("0x{FORK_BLOCK_NUMBER:x}")).await?, remote_balance);
    let nonce: U256 =
        client.request("eth_getTransactionCount", rpc_params![funder, "latest"]).await?;
    assert_eq!(nonce, remote_nonce + U256::from(1));

    // A call that reads the remote balance through the EVM sees the local write.
    let reader = Address::with_last_byte(0xbe);
    client.request::<(), _>("anvil_setCode", rpc_params![reader, balance_of_code(dead)]).await?;
    let call = TransactionRequest::default().with_to(reader);
    let result: Bytes = client.request("eth_call", rpc_params![call, "latest"]).await?;
    assert_eq!(U256::from_be_slice(result.as_ref()), remote_balance + sent);

    // Reverting drops the local write and the remote state shows again.
    let reverted: bool = client.request("evm_revert", rpc_params![snapshot]).await?;
    assert!(reverted);
    assert_eq!(block_number(&client).await?, FORK_BLOCK_NUMBER);
    assert_eq!(balance(&client, dead, "latest").await?, remote_balance);
    let nonce: U256 =
        client.request("eth_getTransactionCount", rpc_params![funder, "latest"]).await?;
    assert_eq!(nonce, remote_nonce);

    Ok(())
}

#[tokio::test]
async fn fork_logs_span_remote_and_local_blocks() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(fork_config()).await?;

    let filter = serde_json::json!({
        "fromBlock": format!("0x{:x}", FORK_BLOCK_NUMBER - 1),
        "toBlock": "latest",
        "address": WETH,
    });
    let logs: Vec<Value> = client.request("eth_getLogs", rpc_params![filter]).await?;
    assert!(!logs.is_empty(), "WETH emits logs in every block");
    assert!(logs.iter().all(|log| {
        log["address"]
            .as_str()
            .is_some_and(|address| address.eq_ignore_ascii_case(&WETH.to_string()))
    }));

    Ok(())
}

#[tokio::test]
async fn dump_state_and_load_state_roundtrip() -> Result<()> {
    let (_api, handle, client) = spawn_with_client(NodeConfig::test()).await?;
    let (funder, gas_price) = funder_and_gas_price(&client).await?;
    let target = Address::with_last_byte(0x42);
    let slot = B256::with_last_byte(7);
    let value = B256::with_last_byte(9);

    client.request::<(), _>("anvil_setBalance", rpc_params![target, U256::from(1234u64)]).await?;
    client.request::<(), _>("anvil_setCode", rpc_params![target, RETURN_42_CODE]).await?;
    client
        .request::<bool, _>("anvil_setStorageAt", rpc_params![target, U256::from(7), value])
        .await?;
    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![transfer(funder, Address::ZERO, gas_price)])
        .await?;
    wait_for_receipt(&client, tx_hash).await?;
    assert_eq!(block_number(&client).await?, 1);

    let dump: Bytes = client.request("anvil_dumpState", rpc_params![]).await?;
    let state = reth_anvil::SerializableState::decode(&dump)?;
    assert_eq!(state.best_block_number, Some(1));
    let record = state.accounts.get(&target).ok_or_eyre("dumped target account")?;
    assert_eq!(record.balance, U256::from(1234u64));
    assert_eq!(record.code, RETURN_42_CODE);
    assert_eq!(record.storage.get(&slot), Some(&value));
    assert_eq!(state.accounts.get(&funder).map(|record| record.nonce), Some(1));
    drop(handle);

    // A node started from the dump continues at the dumped block with the dumped state.
    let (_api, loaded_handle, loaded) =
        spawn_with_client(NodeConfig::test().with_init_state(Some(state.clone()))).await?;
    assert_eq!(block_number(&loaded).await?, 1);
    assert_eq!(balance(&loaded, target, "latest").await?, U256::from(1234u64));
    let code: Bytes = loaded.request("eth_getCode", rpc_params![target, "latest"]).await?;
    assert_eq!(code, RETURN_42_CODE);
    let stored: B256 =
        loaded.request("eth_getStorageAt", rpc_params![target, U256::from(7), "latest"]).await?;
    assert_eq!(stored, value);
    let nonce: U256 =
        loaded.request("eth_getTransactionCount", rpc_params![funder, "latest"]).await?;
    assert_eq!(nonce, U256::from(1));
    let (funder, gas_price) = funder_and_gas_price(&loaded).await?;
    let tx_hash: B256 = loaded
        .request("eth_sendTransaction", rpc_params![transfer(funder, Address::ZERO, gas_price)])
        .await?;
    wait_for_receipt(&loaded, tx_hash).await?;
    assert_eq!(block_number(&loaded).await?, 2);
    drop(loaded_handle);

    // `anvil_loadState` applies the dump on top of a running node.
    let (_api, _handle, fresh) = spawn_with_client(NodeConfig::test()).await?;
    let loaded_ok: bool = fresh.request("anvil_loadState", rpc_params![dump]).await?;
    assert!(loaded_ok);
    assert_eq!(balance(&fresh, target, "latest").await?, U256::from(1234u64));
    let code: Bytes = fresh.request("eth_getCode", rpc_params![target, "latest"]).await?;
    assert_eq!(code, RETURN_42_CODE);
    fresh.request::<(), _>("anvil_mine", rpc_params![]).await?;
    let stored: B256 =
        fresh.request("eth_getStorageAt", rpc_params![target, U256::from(7), "latest"]).await?;
    assert_eq!(stored, value);

    Ok(())
}

/// Regression test for foundry-rs/foundry#17428: anvil returned the intrinsic 21,000 gas for a
/// value transfer to a new account on Amsterdam, which misses the EIP-8037 account creation state
/// gas. Reth executes the transfer at 21,000 gas and only accepts that estimate when it succeeds.
#[tokio::test]
async fn estimate_gas_charges_account_creation_state_gas_on_amsterdam() -> Result<()> {
    let config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Amsterdam.into()));
    let (_api, _handle, client) = spawn_with_client(config).await?;
    let (funder, _) = funder_and_gas_price(&client).await?;
    let fresh = Address::with_last_byte(0xf1);

    let request =
        TransactionRequest::default().with_from(funder).with_to(fresh).with_value(U256::from(1));
    let estimate: U256 = client.request("eth_estimateGas", rpc_params![request]).await?;
    assert!(
        estimate > U256::from(21_000u64),
        "a transfer that creates an account costs more than the intrinsic gas on Amsterdam, got {estimate}"
    );

    // A transfer to an existing account costs no state gas. EIP-2780 lowers the intrinsic gas
    // below 21,000 on Amsterdam.
    let request =
        TransactionRequest::default().with_from(funder).with_to(funder).with_value(U256::from(1));
    let existing: U256 = client.request("eth_estimateGas", rpc_params![request]).await?;
    assert!(existing <= U256::from(21_000u64), "got {existing}");
    assert!(existing < estimate);

    Ok(())
}

#[tokio::test]
async fn eth_send_unsigned_transaction_sends_from_any_account() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let sender = Address::with_last_byte(0xa1);
        let recipient = Address::with_last_byte(0xa2);

        // Fund the sender, which the node does not hold a key for.
        let mut funding = transfer(funder, sender, gas_price);
        funding.value = Some(U256::from(10u64).pow(U256::from(18)));
        let funding_tx: B256 = client.request("eth_sendTransaction", rpc_params![funding]).await?;
        wait_for_receipt(&client, funding_tx).await?;

        let tx_hash: B256 = client
            .request(
                "eth_sendUnsignedTransaction",
                rpc_params![transfer(sender, recipient, gas_price)],
            )
            .await?;
        let receipt = wait_for_receipt(&client, tx_hash).await?;
        assert_eq!(receipt["status"], "0x1");
        assert_eq!(
            receipt["from"].as_str().map(str::to_lowercase),
            Some(sender.to_string().to_lowercase())
        );
        assert_eq!(balance(&client, recipient, "latest").await?, U256::from(1));

        // The account is not left impersonated.
        let err = client
            .request::<B256, _>(
                "eth_sendTransaction",
                rpc_params![transfer(sender, recipient, gas_price)],
            )
            .await
            .expect_err("the sender is not impersonated after the unsigned send");
        assert!(!err.to_string().is_empty());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn impersonate_signature_attributes_raw_transactions() -> Result<()> {
    use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
    use alloy_eips::Encodable2718;
    use alloy_primitives::Signature;

    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let sender = Address::with_last_byte(0xb1);
        let recipient = Address::with_last_byte(0xb2);
        let mut funding = transfer(funder, sender, gas_price);
        funding.value = Some(U256::from(10u64).pow(U256::from(18)));
        let funding_tx: B256 = client.request("eth_sendTransaction", rpc_params![funding]).await?;
        wait_for_receipt(&client, funding_tx).await?;

        let chain_id: U256 = client.request("eth_chainId", rpc_params![]).await?;
        let tx = TxEip1559 {
            chain_id: chain_id.to(),
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: gas_price,
            max_priority_fee_per_gas: 1,
            to: recipient.into(),
            value: U256::from(1),
            ..Default::default()
        };
        // Any well-formed signature does: the override decides the sender.
        let signature = Signature::new(U256::from(1), U256::from(2), false);
        client
            .request::<(), _>(
                "anvil_impersonateSignature",
                rpc_params![Bytes::from(signature.as_bytes().to_vec()), sender],
            )
            .await?;
        let envelope: TxEnvelope = tx.into_signed(signature).into();
        let raw = Bytes::from(envelope.encoded_2718());
        let tx_hash: B256 = client.request("eth_sendRawTransaction", rpc_params![raw]).await?;
        let receipt = wait_for_receipt(&client, tx_hash).await?;
        assert_eq!(receipt["status"], "0x1");
        assert_eq!(
            receipt["from"].as_str().map(str::to_lowercase),
            Some(sender.to_string().to_lowercase())
        );
        assert_eq!(balance(&client, recipient, "latest").await?, U256::from(1));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn personal_sign_and_erigon_header_match_eth_namespace() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, _) = funder_and_gas_price(&client).await?;
        let message = Bytes::from_static(b"reth-anvil");
        let eth_sign: Bytes =
            client.request("eth_sign", rpc_params![funder, message.clone()]).await?;
        let personal: Bytes = client.request("personal_sign", rpc_params![message, funder]).await?;
        assert_eq!(eth_sign, personal);

        let eth_header: Value =
            client.request("eth_getHeaderByNumber", rpc_params!["latest"]).await?;
        let erigon_header: Value =
            client.request("erigon_getHeaderByNumber", rpc_params!["latest"]).await?;
        assert_eq!(eth_header, erigon_header);
        assert_eq!(erigon_header["number"], "0x0");
        Ok(())
    })
    .await
}

async fn mine_two_transfers_with_order(order: reth_anvil::TransactionOrder) -> Result<Vec<String>> {
    let config = NodeConfig::test().with_no_mining(true).with_transaction_order(order);
    let (_api, handle, client) = spawn_with_client(config).await?;
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let (_, gas_price) = funder_and_gas_price(&client).await?;
    let recipient = Address::with_last_byte(0xc1);

    // The first transaction arrives first but pays a lower tip than the second.
    let first: B256 = client
        .request("eth_sendTransaction", rpc_params![transfer(accounts[0], recipient, gas_price)])
        .await?;
    let second: B256 = client
        .request(
            "eth_sendTransaction",
            rpc_params![transfer(accounts[1], recipient, gas_price + 2_000_000_000)],
        )
        .await?;
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    let block = get_block(&client, "latest").await?;
    let hashes: Vec<String> = block["transactions"]
        .as_array()
        .ok_or_eyre("transactions")?
        .iter()
        .filter_map(|hash| hash.as_str().map(str::to_lowercase))
        .collect();
    assert_eq!(hashes.len(), 2);
    let _ = (first, second);
    Ok(vec![
        hashes[0].clone(),
        hashes[1].clone(),
        first.to_string().to_lowercase(),
        second.to_string().to_lowercase(),
    ])
}

#[tokio::test]
async fn transaction_order_fifo_and_fees() -> Result<()> {
    let fifo = mine_two_transfers_with_order(reth_anvil::TransactionOrder::Fifo).await?;
    assert_eq!(&fifo[0..2], &fifo[2..4], "fifo mines in arrival order");

    let fees = mine_two_transfers_with_order(reth_anvil::TransactionOrder::Fees).await?;
    assert_eq!(fees[0], fees[3], "fees mines the higher tip first");
    assert_eq!(fees[1], fees[2]);

    let info: NodeInfo = {
        let (_api, _handle, client) = spawn_with_client(
            NodeConfig::test().with_transaction_order(reth_anvil::TransactionOrder::Fifo),
        )
        .await?;
        client.request("anvil_nodeInfo", rpc_params![]).await?
    };
    assert_eq!(info.transaction_order, "fifo");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn ipc_endpoint_is_created() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("anvil.ipc");
    let config = NodeConfig::test().with_ipc(Some(Some(path.to_string_lossy().into_owned())));
    let (_api, _handle, client) = spawn_with_client(config).await?;
    assert_eq!(block_number(&client).await?, 0);
    assert!(path.exists(), "the ipc socket exists at {}", path.display());
    Ok(())
}

/// Init code that returns 32 KiB of zero bytes as the runtime code: `PUSH3 0x8000 PUSH1 0 RETURN`.
const LARGE_CONTRACT_INIT_CODE: &str = "0x620080006000f3";

/// Runtime code that stores one word at offset 0x1000: `PUSH1 1 PUSH2 0x1000 MSTORE STOP`.
const MSTORE_FAR_CODE: &str = "0x60016110005200";

const U64_MAX_HEX: &str = "0xffffffffffffffff";

#[tokio::test]
async fn disabled_block_gas_limit_mines_above_the_default_limit() -> Result<()> {
    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().disable_block_gas_limit(true)).await?;
    assert_eq!(get_block(&client, "latest").await?["gasLimit"].as_str(), Some(U64_MAX_HEX));

    let (funder, gas_price) = funder_and_gas_price(&client).await?;
    let tx = transfer(funder, Address::repeat_byte(0x33), gas_price).with_gas_limit(30_000_001);
    let tx_hash: B256 = client.request("eth_sendTransaction", rpc_params![tx]).await?;
    let receipt = wait_for_receipt(&client, tx_hash).await?;
    assert_eq!(receipt["status"], "0x1");
    assert_eq!(get_block(&client, "latest").await?["gasLimit"].as_str(), Some(U64_MAX_HEX));
    Ok(())
}

#[tokio::test]
async fn custom_gas_limit_persists_across_blocks() -> Result<()> {
    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_gas_limit(Some(50_000_000))).await?;
    client.request::<(), _>("anvil_mine", rpc_params![U256::from(2), U256::ZERO]).await?;
    let block = get_block(&client, "latest").await?;
    assert_eq!(block["number"].as_str(), Some("0x2"));
    assert_eq!(block["gasLimit"].as_str(), Some("0x2faf080"));
    Ok(())
}

#[tokio::test]
async fn tx_gas_limit_cap_is_enforced_only_when_enabled() -> Result<()> {
    const ABOVE_CAP: u64 = 16_777_217;
    let osaka = || NodeConfig::test().with_hardfork(Some(EthereumHardfork::Osaka.into()));

    let (_api, _handle, client) = spawn_with_client(osaka()).await?;
    let (funder, gas_price) = funder_and_gas_price(&client).await?;
    let tx = transfer(funder, Address::repeat_byte(0x34), gas_price).with_gas_limit(ABOVE_CAP);
    let tx_hash: B256 = client.request("eth_sendTransaction", rpc_params![tx]).await?;
    assert_eq!(wait_for_receipt(&client, tx_hash).await?["status"], "0x1");

    let (_api, _handle, client) = spawn_with_client(osaka().enable_tx_gas_limit(true)).await?;
    let (funder, gas_price) = funder_and_gas_price(&client).await?;
    let tx = transfer(funder, Address::repeat_byte(0x34), gas_price).with_gas_limit(ABOVE_CAP);
    let err = client.request::<B256, _>("eth_sendTransaction", rpc_params![tx]).await.unwrap_err();
    assert!(err.to_string().contains("gas limit"), "{err}");
    Ok(())
}

/// Funds a fresh account and, before the funding is mined, spends from it. Returns the funding
/// hash and the result of submitting the spend.
async fn spend_before_funding(
    client: &HttpClient,
) -> Result<(B256, std::result::Result<B256, ClientError>)> {
    client.request::<(), _>("anvil_setAutomine", rpc_params![false]).await?;
    let (funder, gas_price) = funder_and_gas_price(client).await?;
    let spender = PrivateKeySigner::random();
    let chain_id: U256 = client.request("eth_chainId", rpc_params![]).await?;
    let one_ether = U256::from(10u64).pow(U256::from(18));

    // The funding pays the higher tip, so it is mined first.
    let funding = transfer(funder, spender.address(), gas_price * 2).with_value(one_ether);
    let funding_tx: B256 = client.request("eth_sendTransaction", rpc_params![funding]).await?;

    let mut spend = TxEip1559 {
        chain_id: chain_id.to(),
        nonce: 0,
        gas_limit: 21_000,
        max_fee_per_gas: gas_price,
        max_priority_fee_per_gas: 1,
        to: Address::repeat_byte(0x35).into(),
        value: one_ether / U256::from(2),
        ..Default::default()
    };
    let signature = spender.sign_transaction_sync(&mut spend)?;
    let envelope: TxEnvelope = spend.into_signed(signature).into();
    let raw = Bytes::from(envelope.encoded_2718());
    let spend_tx = client.request::<B256, _>("eth_sendRawTransaction", rpc_params![raw]).await;
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    Ok((funding_tx, spend_tx))
}

#[tokio::test]
async fn pool_balance_checks_can_be_disabled() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(NodeConfig::test()).await?;
    let (_, spend) = spend_before_funding(&client).await?;
    let err = spend.unwrap_err().to_string();
    assert!(err.contains("insufficient funds"), "{err}");

    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_disable_pool_balance_checks(true)).await?;
    let (funding, spend) = spend_before_funding(&client).await?;
    let funding = wait_for_receipt(&client, funding).await?;
    let spend = wait_for_receipt(&client, spend?).await?;
    assert_eq!(spend["status"], "0x1");
    assert_eq!(spend["blockNumber"], funding["blockNumber"]);
    Ok(())
}

async fn deploy_large_contract(client: &HttpClient) -> Result<Value> {
    let (funder, gas_price) = funder_and_gas_price(client).await?;
    let tx = TransactionRequest::default()
        .with_from(funder)
        .with_gas_price(gas_price)
        .with_gas_limit(10_000_000)
        .with_deploy_code(Bytes::from_str(LARGE_CONTRACT_INIT_CODE)?);
    let tx_hash: B256 = client.request("eth_sendTransaction", rpc_params![tx]).await?;
    wait_for_receipt(client, tx_hash).await
}

#[tokio::test]
async fn code_size_limit_can_be_set_and_disabled() -> Result<()> {
    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_code_size_limit(Some(1024))).await?;
    assert_eq!(deploy_large_contract(&client).await?["status"], "0x0");

    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().disable_code_size_limit(true)).await?;
    let receipt = deploy_large_contract(&client).await?;
    assert_eq!(receipt["status"], "0x1");
    let contract = receipt["contractAddress"].as_str().ok_or_eyre("missing contract address")?;
    let code: Bytes = client.request("eth_getCode", rpc_params![contract, "latest"]).await?;
    assert_eq!(code.len(), 0x8000);
    Ok(())
}

async fn call_mstore_far(client: &HttpClient) -> std::result::Result<Bytes, ClientError> {
    let contract = Address::repeat_byte(0x36);
    let overrides = StateOverridesBuilder::default()
        .with_code(contract, Bytes::from_str(MSTORE_FAR_CODE).unwrap())
        .build();
    let call = TransactionRequest::default().with_to(contract);
    client.request("eth_call", rpc_params![call, "latest", overrides]).await
}

#[tokio::test]
async fn memory_limit_applies_to_calls() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(NodeConfig::test()).await?;
    call_mstore_far(&client).await?;

    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_memory_limit(Some(1024))).await?;
    let err = call_mstore_far(&client).await.unwrap_err();
    assert!(err.to_string().to_lowercase().contains("memory"), "{err}");
    Ok(())
}

#[tokio::test]
async fn ws_subscriptions_deliver_new_heads() -> Result<()> {
    let (_api, handle, client) = spawn_with_client(NodeConfig::test()).await?;
    let ws = WsClientBuilder::default().build(handle.ws_endpoint()).await?;
    let mut heads = ws
        .subscribe::<Value, _>("eth_subscribe", rpc_params!["newHeads"], "eth_unsubscribe")
        .await?;
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    let head = tokio::time::timeout(Duration::from_secs(10), heads.next())
        .await?
        .ok_or_eyre("the subscription ended")??;
    assert_eq!(head["number"].as_str(), Some("0x1"));
    let block_number: U256 = ws.request("eth_blockNumber", rpc_params![]).await?;
    assert_eq!(block_number, U256::from(1));
    Ok(())
}

#[tokio::test]
async fn anvil_set_chain_id_relaunches_with_state() -> Result<()> {
    let (api, handle, client) = spawn_with_client(NodeConfig::test()).await?;
    let (funder, gas_price) = funder_and_gas_price(&client).await?;
    let recipient = Address::repeat_byte(0x5F);
    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![transfer(funder, recipient, gas_price)])
        .await?;
    wait_for_receipt(&client, tx_hash).await?;
    client.request::<(), _>("anvil_setBalance", rpc_params![recipient, U256::from(7)]).await?;
    let instance_id = api.instance_id();

    api.anvil_set_chain_id(1234).await?;

    // The endpoint, the in-process api, the state, and the height survive the relaunch.
    assert_eq!(api.chain_id().await?, U256::from(1234));
    let chain_id: U256 = client.request("eth_chainId", rpc_params![]).await?;
    assert_eq!(chain_id, U256::from(1234));
    assert_eq!(handle.http_endpoint(), format!("http://{}", handle.socket_address()));
    assert_eq!(block_number(&client).await?, 1);
    assert_eq!(balance(&client, recipient, "latest").await?, U256::from(7));
    assert_ne!(api.instance_id(), instance_id);

    // The new chain keeps going.
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    assert_eq!(block_number(&client).await?, 2);
    let tx_hash: B256 = client
        .request("eth_sendTransaction", rpc_params![transfer(funder, recipient, gas_price)])
        .await?;
    assert_eq!(wait_for_receipt(&client, tx_hash).await?["status"], "0x1");
    Ok(())
}

#[tokio::test]
async fn anvil_reset_switches_to_another_fork_block() -> Result<()> {
    let (api, _handle, client) = spawn_with_client(fork_config()).await?;
    let instance_id = api.instance_id();
    let earlier = FORK_BLOCK_NUMBER - 10;
    let earlier_header = get_block(&client, format!("0x{earlier:x}")).await?;

    client
        .request::<(), _>(
            "anvil_reset",
            rpc_params![Forking { json_rpc_url: None, block_number: Some(earlier) }],
        )
        .await?;

    assert_eq!(block_number(&client).await?, earlier);
    assert_eq!(get_block(&client, "latest").await?["hash"], earlier_header["hash"]);
    assert_ne!(api.instance_id(), instance_id);
    let info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
    assert_eq!(info.fork_config.fork_block_number, Some(earlier));

    // Mining and a plain reset work on the new fork.
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    assert_eq!(block_number(&client).await?, earlier + 1);
    client.request::<(), _>("anvil_reset", rpc_params![]).await?;
    assert_eq!(block_number(&client).await?, earlier);
    Ok(())
}

#[tokio::test]
async fn custom_chain_id_signs_dev_transactions() -> Result<()> {
    let (_api, _handle, client) =
        spawn_with_client(NodeConfig::test().with_chain_id(Some(99u64))).await?;
    let chain_id: U256 = client.request("eth_chainId", rpc_params![]).await?;
    assert_eq!(chain_id, U256::from(99));
    let (funder, gas_price) = funder_and_gas_price(&client).await?;
    let tx_hash: B256 = client
        .request(
            "eth_sendTransaction",
            rpc_params![transfer(funder, Address::repeat_byte(0x60), gas_price)],
        )
        .await?;
    assert_eq!(wait_for_receipt(&client, tx_hash).await?["status"], "0x1");
    Ok(())
}

/// Every RPC method anvil serves, except the Tempo helpers, `eth_signRawTransaction` (Tempo
/// only), the opcode gas traces, and `eth_callBundle`.
const ANVIL_METHODS: &[&str] = &[
    "anvil_addBalance",
    "anvil_autoImpersonateAccount",
    "anvil_dropAllTransactions",
    "anvil_dropTransaction",
    "anvil_dumpState",
    "anvil_getAutomine",
    "anvil_getBlobByHash",
    "anvil_getBlobsByTransactionHash",
    "anvil_getGenesisTime",
    "anvil_getIntervalMining",
    "anvil_getLastBlockWallTime",
    "anvil_impersonateAccount",
    "anvil_impersonateSignature",
    "anvil_increaseTime",
    "anvil_loadState",
    "anvil_metadata",
    "anvil_mine",
    "anvil_mine_detailed",
    "anvil_nodeInfo",
    "anvil_removeBlockTimestampInterval",
    "anvil_removePoolTransactions",
    "anvil_reorg",
    "anvil_reset",
    "anvil_revert",
    "anvil_rollback",
    "anvil_setAutomine",
    "anvil_setBalance",
    "anvil_setBlockGasLimit",
    "anvil_setBlockTimestampInterval",
    "anvil_setChainId",
    "anvil_setCode",
    "anvil_setCoinbase",
    "anvil_setIntervalMining",
    "anvil_setLoggingEnabled",
    "anvil_setMinGasPrice",
    "anvil_setNextBlockBaseFeePerGas",
    "anvil_setNextBlockParentBeaconBlockRoot",
    "anvil_setNextBlockPrevRandao",
    "anvil_setNextBlockTimestamp",
    "anvil_setNonce",
    "anvil_setRpcUrl",
    "anvil_setStorageAt",
    "anvil_setTime",
    "anvil_snapshot",
    "anvil_stopImpersonatingAccount",
    "debug_accountInfoAt",
    "debug_clearTxpool",
    "debug_codeByHash",
    "debug_dbGet",
    "debug_executionWitness",
    "debug_freeOSMemory",
    "debug_getModifiedAccountsByNumber",
    "debug_getRawBlock",
    "debug_getRawHeader",
    "debug_getRawReceipts",
    "debug_getRawTransaction",
    "debug_getRawTransactions",
    "debug_traceBlock",
    "debug_traceBlockByHash",
    "debug_traceBlockByNumber",
    "debug_traceCall",
    "debug_traceTransaction",
    "erigon_getHeaderByNumber",
    "eth_accounts",
    "eth_baseFee",
    "eth_blobBaseFee",
    "eth_blockNumber",
    "eth_call",
    "eth_callMany",
    "eth_chainId",
    "eth_coinbase",
    "eth_config",
    "eth_createAccessList",
    "eth_estimateGas",
    "eth_feeHistory",
    "eth_fillTransaction",
    "eth_gasPrice",
    "eth_getAccount",
    "eth_getAccountInfo",
    "eth_getBalance",
    "eth_getBlockAccessList",
    "eth_getBlockAccessListByBlockHash",
    "eth_getBlockAccessListByBlockNumber",
    "eth_getBlockAccessListRaw",
    "eth_getBlockByHash",
    "eth_getBlockByNumber",
    "eth_getBlockReceipts",
    "eth_getBlockTransactionCountByHash",
    "eth_getBlockTransactionCountByNumber",
    "eth_getCode",
    "eth_getFilterChanges",
    "eth_getFilterLogs",
    "eth_getHeaderByHash",
    "eth_getHeaderByNumber",
    "eth_getLogs",
    "eth_getProof",
    "eth_getRawTransactionByBlockHashAndIndex",
    "eth_getRawTransactionByBlockNumberAndIndex",
    "eth_getRawTransactionByHash",
    "eth_getStorageAt",
    "eth_getStorageValues",
    "eth_getTransactionByBlockHashAndIndex",
    "eth_getTransactionByBlockNumberAndIndex",
    "eth_getTransactionByHash",
    "eth_getTransactionBySenderAndNonce",
    "eth_getTransactionCount",
    "eth_getTransactionReceipt",
    "eth_getUncleByBlockHashAndIndex",
    "eth_getUncleByBlockNumberAndIndex",
    "eth_getUncleCountByBlockHash",
    "eth_getUncleCountByBlockNumber",
    "eth_getWork",
    "eth_hashrate",
    "eth_maxPriorityFeePerGas",
    "eth_networkId",
    "eth_newBlockFilter",
    "eth_newFilter",
    "eth_newPendingTransactionFilter",
    "eth_pendingTransactions",
    "eth_protocolVersion",
    "eth_requestAccounts",
    "eth_resend",
    "eth_sendRawTransaction",
    "eth_sendRawTransactionConditional",
    "eth_sendRawTransactionSync",
    "eth_sendTransaction",
    "eth_sendTransactionSync",
    "eth_sendUnsignedTransaction",
    "eth_sign",
    "eth_signTransaction",
    "eth_signTypedData",
    "eth_submitHashrate",
    "eth_submitWork",
    "eth_subscribe",
    "eth_syncing",
    "eth_uninstallFilter",
    "eth_unsubscribe",
    "evm_increaseTime",
    "evm_mine",
    "evm_mine_detailed",
    "evm_revert",
    "evm_setAccountNonce",
    "evm_setAutomine",
    "evm_setBlockGasLimit",
    "evm_setIntervalMining",
    "evm_setNextBlockTimestamp",
    "evm_setTime",
    "evm_snapshot",
    "hardhat_addBalance",
    "hardhat_autoImpersonateAccount",
    "hardhat_dropAllTransactions",
    "hardhat_dropTransaction",
    "hardhat_dumpState",
    "hardhat_getAutomine",
    "hardhat_impersonateAccount",
    "hardhat_loadState",
    "hardhat_metadata",
    "hardhat_mine",
    "hardhat_reset",
    "hardhat_setBalance",
    "hardhat_setCode",
    "hardhat_setCoinbase",
    "hardhat_setLoggingEnabled",
    "hardhat_setMinGasPrice",
    "hardhat_setNextBlockBaseFeePerGas",
    "hardhat_setNonce",
    "hardhat_setStorageAt",
    "hardhat_stopImpersonatingAccount",
    "net_listening",
    "net_version",
    "ots_getApiLevel",
    "ots_getBlockDetails",
    "ots_getBlockDetailsByHash",
    "ots_getBlockTransactions",
    "ots_getContractCreator",
    "ots_getInternalOperations",
    "ots_getTransactionBySenderAndNonce",
    "ots_getTransactionError",
    "ots_hasCode",
    "ots_searchTransactionsAfter",
    "ots_searchTransactionsBefore",
    "ots_traceTransaction",
    "personal_sign",
    "tenderly_addBalance",
    "tenderly_setBalance",
    "trace_block",
    "trace_call",
    "trace_callMany",
    "trace_filter",
    "trace_get",
    "trace_rawTransaction",
    "trace_replayBlockTransactions",
    "trace_replayTransaction",
    "trace_transaction",
    "txpool_content",
    "txpool_contentFrom",
    "txpool_inspect",
    "txpool_status",
];

#[tokio::test]
async fn serves_every_anvil_rpc_method() -> Result<()> {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    let served = api.method_names();
    let missing: Vec<_> =
        ANVIL_METHODS.iter().filter(|name| !served.iter().any(|m| m == *name)).collect();
    assert!(missing.is_empty(), "methods anvil serves but this node does not: {missing:?}");
    Ok(())
}

#[tokio::test]
async fn eth_send_transaction_sync_returns_the_receipt() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let receipt: Value = client
            .request(
                "eth_sendTransactionSync",
                rpc_params![transfer(funder, Address::repeat_byte(0x61), gas_price)],
            )
            .await?;
        assert_eq!(receipt["status"], "0x1");
        assert_eq!(receipt["blockNumber"], "0x1");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn eth_resend_replaces_a_pending_transaction() -> Result<()> {
    with_test_client(|client| async move {
        client.request::<(), _>("anvil_setAutomine", rpc_params![false]).await?;
        let (funder, gas_price) = funder_and_gas_price(&client).await?;
        let tx = transfer(funder, Address::repeat_byte(0x62), gas_price).with_nonce(0);
        let first: B256 = client.request("eth_sendTransaction", rpc_params![tx.clone()]).await?;
        let second: B256 = client
            .request("eth_resend", rpc_params![tx, U256::from(gas_price * 2), Option::<u64>::None])
            .await?;
        assert_ne!(first, second);
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let receipt = wait_for_receipt(&client, second).await?;
        assert_eq!(receipt["status"], "0x1");
        assert_eq!(
            U256::from_str(receipt["effectiveGasPrice"].as_str().ok_or_eyre("gas price")?)?,
            U256::from(gas_price * 2)
        );
        assert!(get_receipt(&client, first).await?.is_none());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn fork_at_transaction_hash_replays_the_transactions_before_it() -> Result<()> {
    // The origin mines three transfers in one block.
    let (_origin_api, origin, origin_client) = spawn_with_client(NodeConfig::test()).await?;
    origin_client.request::<(), _>("anvil_setAutomine", rpc_params![false]).await?;
    let (sender, gas_price) = funder_and_gas_price(&origin_client).await?;
    let mut hashes = Vec::new();
    for nonce in 0..3u64 {
        let tx =
            transfer(sender, Address::repeat_byte(0x70 + nonce as u8), gas_price).with_nonce(nonce);
        hashes
            .push(origin_client.request::<B256, _>("eth_sendTransaction", rpc_params![tx]).await?);
    }
    origin_client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    let origin_block = get_block(&origin_client, "0x1").await?;
    assert_eq!(origin_block["transactions"].as_array().map(Vec::len), Some(3));

    // The fork at the second transaction starts with a block that holds the first two.
    let config = NodeConfig::test()
        .with_eth_rpc_url(Some(origin.http_endpoint()))
        .with_fork_transaction_hash(Some(hashes[1]))
        .with_no_mining(true);
    let (_api, _handle, client) = spawn_with_client(config).await?;
    assert_eq!(block_number(&client).await?, 1);
    let block = get_block(&client, "0x1").await?;
    let replayed: Vec<B256> = serde_json::from_value(block["transactions"].clone())?;
    assert_eq!(replayed, hashes[..2]);
    assert_eq!(block["timestamp"], origin_block["timestamp"]);
    assert_eq!(block["miner"], origin_block["miner"]);
    let nonce: U256 =
        client.request("eth_getTransactionCount", rpc_params![sender, "latest"]).await?;
    assert_eq!(nonce, U256::from(2));
    assert_eq!(wait_for_receipt(&client, hashes[1]).await?["status"], "0x1");
    assert!(get_receipt(&client, hashes[2]).await?.is_none());
    assert_eq!(balance(&client, Address::repeat_byte(0x72), "latest").await?, U256::ZERO);

    // Mining continues after the replayed block, and the pool is empty.
    let status: Value = client.request("txpool_status", rpc_params![]).await?;
    assert_eq!(status["pending"], "0x0");
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    assert_eq!(block_number(&client).await?, 2);
    Ok(())
}

#[tokio::test]
async fn eth_call_code_override_keeps_overlay_storage() -> Result<()> {
    with_test_client(|client| async move {
        // `cast call --delegate` overrides the caller's code and reads the caller's storage. The
        // caller has a nonce: revm treats the storage of an account without nonce and code as
        // known to be empty once the account changes, so a code override on a plain address hides
        // the storage anvil wrote to it. Anvil's own state model keeps it.
        let caller = Address::repeat_byte(0xD4);
        let value = B256::from(U256::from(0x1234));
        client.request::<(), _>("anvil_setNonce", rpc_params![caller, U256::ONE]).await?;
        client
            .request::<bool, _>("anvil_setStorageAt", rpc_params![caller, U256::ZERO, value])
            .await?;
        let overrides = StateOverridesBuilder::default().with_code(caller, SLOAD_ZERO_CODE).build();
        let call = TransactionRequest::default().with_to(caller);
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        let storage: B256 =
            client.request("eth_getStorageAt", rpc_params![caller, U256::ZERO, "latest"]).await?;
        assert_eq!(storage, value, "the storage landed in the chain state");
        let result: Bytes = client
            .request("eth_call", rpc_params![call.clone(), "latest", overrides.clone()])
            .await?;
        assert_eq!(B256::from_slice(result.as_ref()), value, "after the write landed");

        let next = B256::from(U256::from(0x5678));
        client
            .request::<bool, _>("anvil_setStorageAt", rpc_params![caller, U256::ZERO, next])
            .await?;
        let result: Bytes =
            client.request("eth_call", rpc_params![call, "latest", overrides]).await?;
        assert_eq!(B256::from_slice(result.as_ref()), next, "from the overlay");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn state_writes_are_visible_at_the_block_they_were_made_at() -> Result<()> {
    with_test_client(|client| async move {
        // Anvil writes into the head state, so a reader at that block sees the write, also after
        // later blocks carried it into the chain state. Tools that replay a block fork at its
        // parent and rely on this.
        let contract = Address::repeat_byte(0xCF);
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        client.request::<(), _>("anvil_setCode", rpc_params![contract, RETURN_42_CODE]).await?;
        client.request::<(), _>("anvil_mine", rpc_params![]).await?;
        assert_eq!(block_number(&client).await?, 2);

        for tag in ["0x1", "0x2", "latest"] {
            let code: Bytes = client.request("eth_getCode", rpc_params![contract, tag]).await?;
            assert_eq!(code, RETURN_42_CODE, "code at {tag}");
        }
        let code: Bytes = client.request("eth_getCode", rpc_params![contract, "0x0"]).await?;
        assert!(code.is_empty(), "the write was made after block 0");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn eth_call_without_gas_gets_the_block_gas_limit() -> Result<()> {
    with_test_client(|client| async move {
        // Runtime code that returns `GAS`.
        let probe = Address::repeat_byte(0xA1);
        client
            .request::<(), _>(
                "anvil_setCode",
                rpc_params![
                    probe,
                    Bytes::from_static(&[0x5a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3])
                ],
            )
            .await?;
        let result: Bytes = client
            .request(
                "eth_call",
                rpc_params![TransactionRequest::default().with_to(probe), "latest"],
            )
            .await?;
        let gas_left = U256::from_be_slice(result.as_ref()).to::<u64>();
        assert!((29_900_000..30_000_000).contains(&gas_left), "gas left: {gas_left}");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn genesis_deploys_the_system_contracts() -> Result<()> {
    with_test_client(|client| async move {
        for address in [
            alloy_eips::eip4788::BEACON_ROOTS_ADDRESS,
            alloy_eips::eip2935::HISTORY_STORAGE_ADDRESS,
            alloy_eips::eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
            alloy_eips::eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
        ] {
            let code: Bytes = client.request("eth_getCode", rpc_params![address, "latest"]).await?;
            assert!(!code.is_empty(), "{address} has code");
        }
        // The history contract records the parent hashes once blocks are mined.
        client.request::<(), _>("anvil_mine", rpc_params![U256::from(2), U256::ZERO]).await?;
        let parent = get_block(&client, "0x1").await?;
        let recorded: B256 = client
            .request(
                "eth_getStorageAt",
                rpc_params![alloy_eips::eip2935::HISTORY_STORAGE_ADDRESS, U256::from(1), "latest"],
            )
            .await?;
        assert_eq!(recorded.to_string(), parent["hash"]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn eth_get_proof_serves_older_blocks() -> Result<()> {
    with_test_client(|client| async move {
        let (funder, _) = funder_and_gas_price(&client).await?;
        client.request::<(), _>("anvil_mine", rpc_params![U256::from(3), U256::ZERO]).await?;
        let proof: Value =
            client.request("eth_getProof", rpc_params![funder, Vec::<U256>::new(), "0x1"]).await?;
        assert_eq!(
            proof["address"].as_str().map(str::to_lowercase),
            Some(funder.to_string().to_lowercase())
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn fork_serves_proofs_for_remote_blocks() -> Result<()> {
    // The origin keeps three blocks; the fork starts at the third and asks for a proof at the
    // first, which only the origin can produce.
    let (_origin_api, origin, origin_client) = spawn_with_client(NodeConfig::test()).await?;
    let (funder, _) = funder_and_gas_price(&origin_client).await?;
    origin_client.request::<(), _>("anvil_mine", rpc_params![U256::from(3), U256::ZERO]).await?;
    let config = NodeConfig::test()
        .with_eth_rpc_url(Some(origin.http_endpoint()))
        .with_fork_block_number(Some(3u64));
    let (_api, _handle, client) = spawn_with_client(config).await?;

    let proof: Value =
        client.request("eth_getProof", rpc_params![funder, Vec::<U256>::new(), "0x1"]).await?;
    assert_eq!(
        proof["address"].as_str().map(str::to_lowercase),
        Some(funder.to_string().to_lowercase())
    );
    assert!(proof["accountProof"].as_array().is_some_and(|nodes| !nodes.is_empty()));

    // At the fork block the account is remote too, and a local write makes it local.
    let proof: Value =
        client.request("eth_getProof", rpc_params![funder, Vec::<U256>::new(), "0x3"]).await?;
    assert!(proof["accountProof"].as_array().is_some_and(|nodes| !nodes.is_empty()));
    client.request::<(), _>("anvil_setBalance", rpc_params![funder, U256::from(5)]).await?;
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    let proof: Value =
        client.request("eth_getProof", rpc_params![funder, Vec::<U256>::new(), "latest"]).await?;
    assert_eq!(proof["balance"].as_str(), Some("0x5"));
    Ok(())
}
