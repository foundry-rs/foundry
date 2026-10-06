//! Integration tests for the Monad network.

use super::{funder_and_gas_price, get_block, spawn_with_client, wait_for_receipt};
use alloy_consensus::{SignableTransaction, TxEip7702, TxEnvelope};
use alloy_eips::{Encodable2718, eip7702::Authorization};
use alloy_network::{TransactionBuilder, TxSignerSync};
use alloy_primitives::{Address, B256, Bytes, U256, address, hex};
use alloy_rpc_types::anvil::NodeInfo;
use alloy_rpc_types_eth::{
    TransactionRequest,
    simulate::{SimBlock, SimulatePayload, SimulatedBlock},
};
use alloy_signer::SignerSync;
use alloy_signer_local::{MnemonicBuilder, PrivateKeySigner, coins_bip39::English};
use eyre::{OptionExt, Result};
use foundry_evm_hardforks::MonadHardfork;
use jsonrpsee::{core::client::ClientT, http_client::HttpClient, rpc_params};
use monad_revm::{cfg::MONAD_TX_GAS_LIMIT_CAP, reserve_balance::abi::RESERVE_BALANCE_ADDRESS};
use reth_anvil::NodeConfig;

const RESERVE_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002000");
const STORAGE_GAS_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002004");
const DIPPED_INTO_RESERVE_SELECTOR: [u8; 4] = hex!("3a61584e");
/// Calls `dippedIntoReserve()` on the reserve balance precompile and returns its result.
const RESERVE_RETURN_PROBE_CODE: [u8; 25] =
    hex!("633a61584e5f5260205f6004601c5f6110015af15060205ff3");
/// Calls `dippedIntoReserve()` and stores the result at the slot named by the calldata.
const RESERVE_STORE_PROBE_CODE: [u8; 27] =
    hex!("633a61584e5f5260205f6004601c5f6110015af1505f515f355500");
const EIP170_CODE_SIZE_LIMIT: usize = 0x6000;
const EIP3860_INITCODE_SIZE_LIMIT: usize = 0xc000;
const EIP7825_TX_GAS_LIMIT_CAP: u64 = 0x100_0000;

fn monad_config(hardfork: MonadHardfork) -> NodeConfig {
    NodeConfig::test_monad().with_hardfork(Some(hardfork.into()))
}

fn mon(value: u64) -> U256 {
    U256::from(value) * U256::from(10u64).pow(U256::from(18))
}

/// Init code that copies `runtime_len` zero bytes after itself into the runtime code.
fn large_contract_init_code(runtime_len: usize) -> Bytes {
    const HEADER_LEN: usize = 15;
    let len = u16::try_from(runtime_len).expect("runtime fits u16");
    let mut code = Vec::with_capacity(HEADER_LEN + runtime_len);
    code.extend_from_slice(&[0x61, (len >> 8) as u8, len as u8]);
    code.extend_from_slice(&[0x61, 0x00, HEADER_LEN as u8]);
    code.extend_from_slice(&[0x60, 0x00, 0x39]);
    code.extend_from_slice(&[0x61, (len >> 8) as u8, len as u8]);
    code.extend_from_slice(&[0x60, 0x00, 0xf3]);
    code.resize(HEADER_LEN + runtime_len, 0);
    Bytes::from(code)
}

/// Returns the gas two cold reads of slot 0 and `second_slot` cost.
fn storage_read_probe_code(second_slot: u8) -> Bytes {
    let mut code = hex!("5a5f5450600054505a90035f5260205ff3");
    code[5] = second_slot;
    Bytes::from(code)
}

/// Returns the gas two writes to slot 0 and `second_slot` cost.
fn storage_write_probe_code(second_slot: u8) -> Bytes {
    let mut code = hex!("5a60015f5560016000555a90035f5260205ff3");
    code[8] = second_slot;
    Bytes::from(code)
}

async fn call(client: &HttpClient, tx: TransactionRequest) -> Result<Bytes> {
    Ok(client.request("eth_call", rpc_params![tx, "latest"]).await?)
}

async fn probe_gas(client: &HttpClient, code: Bytes) -> Result<u64> {
    client.request::<(), _>("anvil_setCode", rpc_params![STORAGE_GAS_PROBE_ADDRESS, code]).await?;
    let result =
        call(client, TransactionRequest::default().with_to(STORAGE_GAS_PROBE_ADDRESS)).await?;
    Ok(U256::from_be_slice(&result).to::<u64>())
}

async fn send(client: &HttpClient, tx: TransactionRequest) -> Result<serde_json::Value> {
    let tx_hash: B256 = client.request("eth_sendTransaction", rpc_params![tx]).await?;
    wait_for_receipt(client, tx_hash).await
}

#[tokio::test]
async fn monad_node_info_reports_network_and_hardfork() -> Result<()> {
    for (config, hardfork) in [
        (NodeConfig::test_monad(), MonadHardfork::default()),
        (monad_config(MonadHardfork::MonadNine), MonadHardfork::MonadNine),
    ] {
        let (_api, _handle, client) = spawn_with_client(config).await?;
        let info: NodeInfo = client.request("anvil_nodeInfo", rpc_params![]).await?;
        assert_eq!(info.network.as_deref(), Some("monad"));
        assert_eq!(info.hard_fork, hardfork.to_string());
        assert_eq!(get_block(&client, "latest").await?["number"].as_str(), Some("0x0"));
    }
    Ok(())
}

#[tokio::test]
async fn monad_nine_exposes_reserve_balance_precompile_for_calls() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(monad_config(MonadHardfork::MonadNine)).await?;
    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    assert_eq!(call(&client, tx).await?, Bytes::from(vec![0; 32]));
    Ok(())
}

#[tokio::test]
async fn monad_ten_applies_mip8_storage_gas() -> Result<()> {
    for (hardfork, read_delta, write_delta) in
        [(MonadHardfork::MonadNine, 0, 0), (MonadHardfork::MonadTen, 8_000, 10_800)]
    {
        let (_api, _handle, client) = spawn_with_client(monad_config(hardfork)).await?;
        let same_page_read = probe_gas(&client, storage_read_probe_code(127)).await?;
        let other_page_read = probe_gas(&client, storage_read_probe_code(128)).await?;
        assert_eq!(other_page_read - same_page_read, read_delta, "{hardfork} read delta");
        let same_page_write = probe_gas(&client, storage_write_probe_code(1)).await?;
        let other_page_write = probe_gas(&client, storage_write_probe_code(128)).await?;
        assert_eq!(other_page_write - same_page_write, write_delta, "{hardfork} write delta");
    }
    Ok(())
}

#[tokio::test]
async fn monad_call_uses_parent_sender_context() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(monad_config(MonadHardfork::MonadNine)).await?;
    let (sender, _) = funder_and_gas_price(&client).await?;
    client
        .request::<(), _>(
            "anvil_setCode",
            rpc_params![RESERVE_PROBE_ADDRESS, Bytes::from(RESERVE_RETURN_PROBE_CODE)],
        )
        .await?;
    client.request::<(), _>("anvil_setBalance", rpc_params![sender, mon(13)]).await?;

    let probe = |value: U256| {
        TransactionRequest::default()
            .with_from(sender)
            .with_to(RESERVE_PROBE_ADDRESS)
            .with_value(value)
            .with_gas_limit(100_000)
    };
    assert_eq!(send(&client, probe(mon(1))).await?["status"], "0x1");

    // The sender is in the parent block now, so this call dips into the reserve.
    let result = call(&client, probe(mon(3))).await?;
    assert_eq!(result, Bytes::from(U256::ONE.to_be_bytes::<32>()));
    Ok(())
}

#[tokio::test]
async fn monad_can_mine_contracts_above_the_ethereum_size_limits() -> Result<()> {
    let (_api, _handle, client) =
        spawn_with_client(monad_config(MonadHardfork::MonadEight)).await?;
    let (from, _) = funder_and_gas_price(&client).await?;

    for (runtime_len, gas) in
        [(EIP170_CODE_SIZE_LIMIT + 1, 10_000_000), (EIP3860_INITCODE_SIZE_LIMIT, 25_000_000)]
    {
        let init_code = large_contract_init_code(runtime_len);
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_deploy_code(init_code)
            .with_gas_limit(gas);
        let receipt = send(&client, tx).await?;
        assert_eq!(receipt["status"], "0x1", "deploying {runtime_len} bytes");
        let contract = receipt["contractAddress"].as_str().ok_or_eyre("contract address")?;
        let code: Bytes = client.request("eth_getCode", rpc_params![contract, "latest"]).await?;
        assert_eq!(code.len(), runtime_len);
    }
    Ok(())
}

#[tokio::test]
async fn monad_tx_gas_limit_cap_replaces_the_ethereum_cap() -> Result<()> {
    let config = || {
        monad_config(MonadHardfork::MonadEight)
            .enable_tx_gas_limit(true)
            .with_gas_limit(Some(40_000_000))
    };
    let (_api, _handle, client) = spawn_with_client(config()).await?;
    let (from, _) = funder_and_gas_price(&client).await?;
    let transfer = |gas_limit: u64| {
        TransactionRequest::default()
            .with_from(from)
            .with_to(Address::repeat_byte(0x41))
            .with_value(U256::ONE)
            .with_gas_limit(gas_limit)
    };

    let receipt = send(&client, transfer(EIP7825_TX_GAS_LIMIT_CAP + 1)).await?;
    assert_eq!(receipt["status"], "0x1");

    let err = client
        .request::<B256, _>(
            "eth_sendTransaction",
            rpc_params![transfer(MONAD_TX_GAS_LIMIT_CAP + 1)],
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("gas limit"), "{err}");
    Ok(())
}

#[tokio::test]
async fn monad_pool_charges_only_the_effective_fee_up_front() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(monad_config(MonadHardfork::MonadNine)).await?;
    let accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let gas_limit = 21_000u64;
    let base_fee = 1_000_000_000u128;
    let priority_fee = 1_000_000_000u128;
    let max_fee = 100_000_000_000u128;
    let effective_fee = U256::from(gas_limit) * U256::from(base_fee + priority_fee);

    let transfer = |value: U256| {
        TransactionRequest::default()
            .with_from(accounts[0])
            .with_to(accounts[1])
            .with_value(value)
            .with_gas_limit(gas_limit)
            .with_max_fee_per_gas(max_fee)
            .with_max_priority_fee_per_gas(priority_fee)
    };

    // The balance covers the fee at the effective price, not at the maximum fee.
    client
        .request::<(), _>("anvil_setNextBlockBaseFeePerGas", rpc_params![U256::from(base_fee)])
        .await?;
    client.request::<(), _>("anvil_setBalance", rpc_params![accounts[0], effective_fee]).await?;
    assert_eq!(send(&client, transfer(U256::ZERO)).await?["status"], "0x1");

    // A value the balance does not cover is admitted, and the transaction fails when it runs.
    client
        .request::<(), _>("anvil_setNextBlockBaseFeePerGas", rpc_params![U256::from(base_fee)])
        .await?;
    client.request::<(), _>("anvil_setBalance", rpc_params![accounts[0], effective_fee]).await?;
    let recipient_balance: U256 =
        client.request("eth_getBalance", rpc_params![accounts[1], "latest"]).await?;
    assert_eq!(send(&client, transfer(U256::ONE)).await?["status"], "0x0");
    let after: U256 = client.request("eth_getBalance", rpc_params![accounts[1], "latest"]).await?;
    assert_eq!(after, recipient_balance);
    Ok(())
}

/// The dev wallets of the default test mnemonic.
fn dev_wallets(count: u32) -> Vec<PrivateKeySigner> {
    (0..count)
        .map(|index| {
            MnemonicBuilder::<English>::default()
                .phrase(reth_anvil::DEFAULT_MNEMONIC)
                .index(index)
                .expect("valid index")
                .build()
                .expect("valid wallet")
        })
        .collect()
}

async fn storage(client: &HttpClient, address: Address, slot: u64) -> Result<U256> {
    let value: B256 = client
        .request("eth_getStorageAt", rpc_params![address, U256::from(slot), "latest"])
        .await?;
    Ok(value.into())
}

fn reserve_probe_tx(from: Address, nonce: u64, slot: u64, value: U256) -> TransactionRequest {
    TransactionRequest::default()
        .with_from(from)
        .with_to(RESERVE_PROBE_ADDRESS)
        .with_nonce(nonce)
        .with_value(value)
        .with_gas_limit(100_000)
        .with_input(Bytes::from(U256::from(slot).to_be_bytes::<32>()))
}

#[tokio::test]
async fn monad_mining_tracks_current_and_ancestor_senders() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(monad_config(MonadHardfork::MonadNine)).await?;
    let accounts: Vec<Address> = client.request("eth_accounts", rpc_params![]).await?;
    let (parent_sender, grandparent_sender, current_sender) =
        (accounts[0], accounts[1], accounts[2]);
    let (first_value, second_value) = (mon(2), mon(1));

    client
        .request::<(), _>(
            "anvil_setCode",
            rpc_params![RESERVE_PROBE_ADDRESS, Bytes::from(RESERVE_STORE_PROBE_CODE)],
        )
        .await?;
    for sender in [parent_sender, grandparent_sender, current_sender] {
        client.request::<(), _>("anvil_setBalance", rpc_params![sender, mon(12)]).await?;
    }

    // A sender dips into the reserve once, and not again while it is in the two ancestor blocks.
    send(&client, reserve_probe_tx(parent_sender, 0, 0, first_value)).await?;
    assert_eq!(storage(&client, RESERVE_PROBE_ADDRESS, 0).await?, U256::ZERO);
    send(&client, reserve_probe_tx(parent_sender, 1, 1, second_value)).await?;
    assert_eq!(storage(&client, RESERVE_PROBE_ADDRESS, 1).await?, U256::ONE);

    send(&client, reserve_probe_tx(grandparent_sender, 0, 2, first_value)).await?;
    assert_eq!(storage(&client, RESERVE_PROBE_ADDRESS, 2).await?, U256::ZERO);
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    send(&client, reserve_probe_tx(grandparent_sender, 1, 3, second_value)).await?;
    assert_eq!(storage(&client, RESERVE_PROBE_ADDRESS, 3).await?, U256::ONE);

    // Within one block, the earlier transaction counts as well.
    client.request::<(), _>("anvil_setAutomine", rpc_params![false]).await?;
    let first: B256 = client
        .request(
            "eth_sendTransaction",
            rpc_params![reserve_probe_tx(current_sender, 0, 4, first_value)],
        )
        .await?;
    let second: B256 = client
        .request(
            "eth_sendTransaction",
            rpc_params![reserve_probe_tx(current_sender, 1, 5, second_value)],
        )
        .await?;
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    assert_eq!(
        wait_for_receipt(&client, first).await?["blockNumber"],
        wait_for_receipt(&client, second).await?["blockNumber"]
    );
    assert_eq!(storage(&client, RESERVE_PROBE_ADDRESS, 4).await?, U256::ZERO);
    assert_eq!(storage(&client, RESERVE_PROBE_ADDRESS, 5).await?, U256::ONE);
    Ok(())
}

#[tokio::test]
async fn monad_mining_tracks_eip7702_authorities() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(monad_config(MonadHardfork::MonadNine)).await?;
    let wallets = dev_wallets(3);
    let authority = &wallets[0];
    let chain_id: U256 = client.request("eth_chainId", rpc_params![]).await?;
    let chain_id = chain_id.to::<u64>();

    client
        .request::<(), _>(
            "anvil_setCode",
            rpc_params![RESERVE_PROBE_ADDRESS, Bytes::from(RESERVE_STORE_PROBE_CODE)],
        )
        .await?;
    client.request::<(), _>("anvil_setBalance", rpc_params![authority.address(), mon(12)]).await?;

    // Wallet 1 sends a transaction that carries an authorization signed by wallet 0.
    let authorization =
        Authorization { chain_id: U256::from(chain_id), address: Address::ZERO, nonce: 0 };
    let signature = authority.sign_hash_sync(&authorization.signature_hash())?;
    let mut tx = TxEip7702 {
        chain_id,
        nonce: 0,
        gas_limit: 100_000,
        max_fee_per_gas: 2_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: wallets[2].address(),
        authorization_list: vec![authorization.into_signed(signature)],
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut tx)?;
    let envelope: TxEnvelope = tx.into_signed(signature).into();
    client.request::<(), _>("anvil_setAutomine", rpc_params![false]).await?;
    let authorization_tx: B256 = client
        .request("eth_sendRawTransaction", rpc_params![Bytes::from(envelope.encoded_2718())])
        .await?;

    // In the same block, the authority cannot dip into its reserve. The pool holds back a signed
    // transaction with the nonce the authorization creates, so the probe is impersonated.
    client.request::<(), _>("anvil_impersonateAccount", rpc_params![authority.address()]).await?;
    let probe = reserve_probe_tx(authority.address(), 1, 6, mon(3))
        .with_max_fee_per_gas(2_000_000_000)
        .with_max_priority_fee_per_gas(1);
    let probe_tx: B256 = client.request("eth_sendTransaction", rpc_params![probe]).await?;
    client.request::<(), _>("anvil_mine", rpc_params![]).await?;
    assert_eq!(wait_for_receipt(&client, authorization_tx).await?["status"], "0x1");
    assert_eq!(wait_for_receipt(&client, probe_tx).await?["status"], "0x1");
    assert_eq!(storage(&client, RESERVE_PROBE_ADDRESS, 6).await?, U256::ONE);
    Ok(())
}

#[tokio::test]
async fn monad_simulate_tracks_senders_within_and_across_blocks() -> Result<()> {
    let (_api, _handle, client) = spawn_with_client(monad_config(MonadHardfork::MonadNine)).await?;
    let (sender, _) = funder_and_gas_price(&client).await?;
    client
        .request::<(), _>(
            "anvil_setCode",
            rpc_params![RESERVE_PROBE_ADDRESS, Bytes::from(RESERVE_RETURN_PROBE_CODE)],
        )
        .await?;
    client.request::<(), _>("anvil_setBalance", rpc_params![sender, mon(12)]).await?;
    let request = |value: U256| {
        TransactionRequest::default()
            .with_from(sender)
            .with_to(RESERVE_PROBE_ADDRESS)
            .with_value(value)
            .with_gas_limit(100_000)
    };
    let zero = Bytes::from(U256::ZERO.to_be_bytes::<32>());
    let one = Bytes::from(U256::ONE.to_be_bytes::<32>());

    // The second call in a block sees the first call's sender.
    let payload = SimulatePayload {
        block_state_calls: vec![SimBlock {
            calls: vec![request(mon(2)), request(mon(1))],
            ..Default::default()
        }],
        ..Default::default()
    };
    let blocks: Vec<SimulatedBlock> =
        client.request("eth_simulateV1", rpc_params![payload, "latest"]).await?;
    assert_eq!(blocks[0].calls[0].return_data, zero);
    assert_eq!(blocks[0].calls[1].return_data, one);

    // Two empty blocks later, the sender has aged out of the ancestor set.
    let payload = SimulatePayload {
        block_state_calls: vec![
            SimBlock { calls: vec![request(mon(2))], ..Default::default() },
            SimBlock::default(),
            SimBlock::default(),
            SimBlock { calls: vec![request(mon(1))], ..Default::default() },
        ],
        ..Default::default()
    };
    let blocks: Vec<SimulatedBlock> =
        client.request("eth_simulateV1", rpc_params![payload, "latest"]).await?;
    assert_eq!(blocks[0].calls[0].return_data, zero);
    assert_eq!(blocks[3].calls[0].return_data, zero);
    Ok(())
}
