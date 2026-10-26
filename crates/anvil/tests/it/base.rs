//! Tests for Base chain support.

use crate::utils::http_provider_with_signer;
use alloy_consensus::{Sealed, Typed2718};
use alloy_eips::{Decodable2718, Encodable2718};
use alloy_network::{EthereumWallet, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, address, b256, keccak256};
use alloy_provider::{
    Provider,
    ext::{DebugApi, TxPoolApi},
};
use alloy_rpc_types::{
    BlockId, TransactionRequest,
    anvil::Forking,
    trace::geth::{
        CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingCallOptions,
        GethDebugTracingOptions, GethTrace,
    },
};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, sol};
use anvil::{NodeConfig, spawn};
use base_common_consensus::{
    BaseTimeDepositSource, Call, DepositSourceDomain, Eip8130Constants, Eip8130Contracts,
    Eip8130Signed, Predeploys, SystemAddresses, TxEip8130,
};
use base_common_evm::BaseTime;
use base_common_precompiles::NonceManagerStorage;
use base_protocol::{BaseTimeUpdateTx, L1BlockInfoTx};
use foundry_common::provider::RetryProvider;
use foundry_config::Config;
use foundry_evm::{hardforks::BaseUpgrade, opts::EvmOpts};
use foundry_evm_networks::NetworkConfigs;
use foundry_primitives::FoundryTxEnvelope;
use op_alloy_consensus::TxDeposit;
use std::time::Duration;

const ACTIVATION_REGISTRY: Address = address!("8453000000000000000000000000000000000001");
const MAINNET_BERYL_ACTIVATION_ADMIN: Address =
    address!("ce3a3bee7e72e2a24079f3c0cb3b97740ed425a9");
const NONCE_MANAGER: Address = address!("813000000000000000000000000000000000aa01");
const BASE_CREATE2_DEPLOYER: Address = address!("13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2");

sol! {
    interface IBaseTime {
        error BaseTime_InvalidTimestampMillisPart();
        error BaseTime_NotDepositor();

        function timestampMillisPart() external view returns (uint16);
        function timestampMs() external view returns (uint64);
        function setTimestampMillisPart(uint16 timestampMillisPart) external;
    }
}

fn eip8130_envelope_with(
    signer: &PrivateKeySigner,
    calls: Vec<Vec<Call>>,
    metadata: Bytes,
) -> FoundryTxEnvelope {
    let tx = TxEip8130 {
        chain_id: 8453,
        sender: None,
        nonce_key: U256::ZERO,
        nonce_sequence: 0,
        valid_after: 0,
        valid_before: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 1_000_000_000,
        gas_limit: 200_000,
        account_changes: Vec::new(),
        calls,
        metadata,
        payer: None,
    };
    let signature = signer.sign_hash_sync(&tx.sender_signature_hash()).unwrap();
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(tx, signature.as_bytes().into(), Bytes::new()))
}

fn eip8130_envelope(signer: &PrivateKeySigner) -> FoundryTxEnvelope {
    eip8130_envelope_with(signer, Vec::new(), Bytes::new())
}

fn malformed_configured_eip8130_envelope(signer: &PrivateKeySigner) -> FoundryTxEnvelope {
    malformed_configured_eip8130_envelope_with_nonce(signer, U256::ZERO, 0)
}

fn malformed_configured_eip8130_envelope_with_nonce(
    signer: &PrivateKeySigner,
    nonce_key: U256,
    nonce_sequence: u64,
) -> FoundryTxEnvelope {
    let tx = TxEip8130 {
        chain_id: 8453,
        sender: Some(signer.address()),
        nonce_key,
        nonce_sequence,
        valid_after: 0,
        valid_before: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 1_000_000_000,
        gas_limit: 200_000,
        account_changes: Vec::new(),
        calls: Vec::new(),
        metadata: Bytes::new(),
        payer: None,
    };
    let bare_auth = signer.sign_hash_sync(&tx.sender_signature_hash()).unwrap().as_bytes().into();
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(tx, bare_auth, Bytes::new()))
}

fn eip8130_envelope_with_nonce(
    signer: &PrivateKeySigner,
    nonce_key: U256,
    nonce_sequence: u64,
    valid_before: u64,
) -> FoundryTxEnvelope {
    eip8130_envelope_with_nonce_and_fee(
        signer,
        nonce_key,
        nonce_sequence,
        valid_before,
        1_000_000_000,
    )
}

fn eip8130_envelope_with_nonce_and_fee(
    signer: &PrivateKeySigner,
    nonce_key: U256,
    nonce_sequence: u64,
    valid_before: u64,
    max_fee_per_gas: u128,
) -> FoundryTxEnvelope {
    let tx = TxEip8130 {
        chain_id: 8453,
        sender: None,
        nonce_key,
        nonce_sequence,
        valid_after: 0,
        valid_before,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas,
        gas_limit: 200_000,
        account_changes: Vec::new(),
        calls: Vec::new(),
        metadata: Bytes::new(),
        payer: None,
    };
    let signature = signer.sign_hash_sync(&tx.sender_signature_hash()).unwrap();
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(tx, signature.as_bytes().into(), Bytes::new()))
}

fn eip8130_envelope_with_channel_calls(
    signer: &PrivateKeySigner,
    nonce_key: U256,
    calls: Vec<Vec<Call>>,
) -> FoundryTxEnvelope {
    let tx = TxEip8130 {
        chain_id: 8453,
        sender: None,
        nonce_key,
        nonce_sequence: 0,
        valid_after: 0,
        valid_before: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 1_000_000_000,
        gas_limit: 200_000,
        account_changes: Vec::new(),
        calls,
        metadata: Bytes::new(),
        payer: None,
    };
    let signature = signer.sign_hash_sync(&tx.sender_signature_hash()).unwrap();
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(tx, signature.as_bytes().into(), Bytes::new()))
}

fn sponsored_eip8130_envelope(
    sender: &PrivateKeySigner,
    payer: &PrivateKeySigner,
) -> FoundryTxEnvelope {
    sponsored_eip8130_envelope_with_nonce(sender, payer, U256::ZERO, 0)
}

fn sponsored_eip8130_envelope_with_nonce(
    sender: &PrivateKeySigner,
    payer: &PrivateKeySigner,
    nonce_key: U256,
    nonce_sequence: u64,
) -> FoundryTxEnvelope {
    let tx = TxEip8130 {
        chain_id: 8453,
        sender: None,
        nonce_key,
        nonce_sequence,
        valid_after: 0,
        valid_before: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 1_000_000_000,
        gas_limit: 200_000,
        account_changes: Vec::new(),
        calls: Vec::new(),
        metadata: Bytes::new(),
        payer: Some(payer.address()),
    };
    let sender_auth =
        sender.sign_hash_sync(&tx.sender_signature_hash()).unwrap().as_bytes().to_vec();
    let payer_signature = payer.sign_hash_sync(&tx.payer_signature_hash(sender.address())).unwrap();
    let mut payer_auth = Eip8130Constants::K1_AUTHENTICATOR.to_vec();
    payer_auth.extend_from_slice(&payer_signature.as_bytes());
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(tx, sender_auth.into(), payer_auth.into()))
}

fn eip8130_simulation_request(sender: Address) -> WithOtherFields<TransactionRequest> {
    serde_json::from_value(serde_json::json!({
        "from": sender,
        "type": "0x79",
        "calls": [],
        "maxFeePerGas": "0x3b9aca00",
        "gas": "0x30d40"
    }))
    .unwrap()
}

fn eip8130_simulation_request_with_call(
    sender: Address,
    target: Address,
) -> WithOtherFields<TransactionRequest> {
    serde_json::from_value(serde_json::json!({
        "from": sender,
        "calls": [[{ "to": target, "data": "0x" }]],
        "maxFeePerGas": "0x3b9aca00",
        "gas": "0x30d40"
    }))
    .unwrap()
}

fn eip8130_auth_blob(authenticator: Address, data_len: usize) -> Bytes {
    let mut blob = authenticator.to_vec();
    blob.resize(blob.len() + data_len, 0xff);
    blob.into()
}

#[tokio::test(flavor = "multi_thread")]
async fn base_node_info_and_call_use_native_evm() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network.as_deref(), Some("base"));
    assert_eq!(node_info.hard_fork, "Beryl");

    let selector = &keccak256("admin()")[..4];
    let output = provider
        .call(
            TransactionRequest::default()
                .with_to(ACTIVATION_REGISTRY)
                .with_input(Bytes::copy_from_slice(selector))
                .into(),
        )
        .await
        .unwrap();
    assert_eq!(output.len(), 32);
    assert_eq!(Address::from_slice(&output[12..]), MAINNET_BERYL_ACTIVATION_ADMIN);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_activation_admin_override_is_used() {
    let admin = Address::repeat_byte(0xaa);
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Beryl.into()))
        .with_base_activation_admin(Some(admin));
    let (_api, handle) = spawn(config).await;
    let output = handle
        .http_provider()
        .call(
            TransactionRequest::default()
                .with_to(ACTIVATION_REGISTRY)
                .with_input(Bytes::copy_from_slice(&keccak256("admin()")[..4]))
                .into(),
        )
        .await
        .unwrap();

    assert_eq!(Address::from_slice(&output[12..]), admin);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_azul_excludes_beryl_precompiles() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Azul.into()));
    let (_api, handle) = spawn(config).await;
    let output = handle
        .http_provider()
        .call(
            TransactionRequest::default()
                .with_to(ACTIVATION_REGISTRY)
                .with_input(Bytes::copy_from_slice(&keccak256("admin()")[..4]))
                .into(),
        )
        .await
        .unwrap();

    assert!(output.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_anvil_node_info_infers_native_network() {
    let (_api, handle) = spawn(NodeConfig::test_base()).await;
    let mut evm_opts = Config::figment().extract::<EvmOpts>().unwrap();
    evm_opts.fork_url = Some(handle.http_endpoint());
    assert_eq!(evm_opts.networks, NetworkConfigs::default());

    evm_opts.infer_network_from_fork().await.unwrap();

    assert!(evm_opts.networks.is_base());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_fork_call_and_trace_use_native_evm() {
    let source_config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (source_api, source_handle) = spawn(source_config).await;
    source_api.mine_one().await.unwrap();
    let target_config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Beryl.into()))
        .with_eth_rpc_url(Some(source_handle.http_endpoint()));
    let (_target_api, target_handle) = spawn(target_config).await;
    let provider = target_handle.http_provider();
    let selector = Bytes::copy_from_slice(&keccak256("admin()")[..4]);
    let call = TransactionRequest::default().with_to(ACTIVATION_REGISTRY).with_input(selector);

    let output = provider.call(WithOtherFields::new(call.clone())).await.unwrap();
    assert_eq!(Address::from_slice(&output[12..]), MAINNET_BERYL_ACTIVATION_ADMIN);

    let trace = provider
        .debug_trace_call(
            WithOtherFields::new(call),
            BlockId::latest(),
            GethDebugTracingCallOptions::default(),
        )
        .await
        .unwrap();
    let GethTrace::Default(frame) = trace else { panic!("expected default trace") };
    assert!(!frame.failed);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_mines_ordinary_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (_api, handle) = spawn(config).await;
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let signer: EthereumWallet = accounts[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);
    let value = U256::from(1_234);
    let before = provider.get_balance(to).await.unwrap();

    let receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(from)
                .with_to(to)
                .with_value(value)
                .with_gas_limit(21_000)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());
    assert_eq!(receipt.from(), from);
    assert_eq!(receipt.to(), Some(to));
    assert_eq!(provider.get_balance(to).await.unwrap(), before + value);
    assert!(provider.get_balance(Predeploys::L1_FEE_VAULT).await.unwrap() > U256::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_denim_initializes_base_time() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    assert!(!provider.get_code_at(Predeploys::BASE_TIME).await.unwrap().is_empty());

    let millis_part = provider
        .call(
            TransactionRequest::default()
                .with_to(Predeploys::BASE_TIME)
                .with_input(IBaseTime::timestampMillisPartCall {}.abi_encode())
                .into(),
        )
        .block(BlockId::latest())
        .await
        .unwrap();
    assert_eq!(IBaseTime::timestampMillisPartCall::abi_decode_returns(&millis_part).unwrap(), 0);

    // Mine through the protocol path. The BaseTime deposit is sent by the canonical depositor,
    // whose nonce must not be reused by a test transaction.
    api.mine_one().await.unwrap();

    let millis_part = provider
        .call(
            TransactionRequest::default()
                .with_to(Predeploys::BASE_TIME)
                .with_input(IBaseTime::timestampMillisPartCall {}.abi_encode())
                .into(),
        )
        .block(BlockId::latest())
        .await
        .unwrap();
    assert_eq!(IBaseTime::timestampMillisPartCall::abi_decode_returns(&millis_part).unwrap(), 200);

    let timestamp_ms = provider
        .call(
            TransactionRequest::default()
                .with_to(Predeploys::BASE_TIME)
                .with_input(IBaseTime::timestampMsCall {}.abi_encode())
                .into(),
        )
        .block(BlockId::latest())
        .await
        .unwrap();
    let timestamp_ms = IBaseTime::timestampMsCall::abi_decode_returns(&timestamp_ms).unwrap();
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(timestamp_ms, block.header.timestamp * 1_000 + 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_denim_mines_base_time_updates() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    for expected_millis_part in [200, 400, 600, 800, 0] {
        api.mine_one().await.unwrap();
        let output = provider
            .call(
                TransactionRequest::default()
                    .with_to(Predeploys::BASE_TIME)
                    .with_input(IBaseTime::timestampMillisPartCall {}.abi_encode())
                    .into(),
            )
            .block(BlockId::latest())
            .await
            .unwrap();
        assert_eq!(
            IBaseTime::timestampMillisPartCall::abi_decode_returns(&output).unwrap(),
            expected_millis_part
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_denim_system_transactions_are_valid() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    api.mine_one().await.unwrap();

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    let hashes = block.transactions.hashes().collect::<Vec<_>>();
    assert_eq!(hashes.len(), 2, "Denim blocks contain the L1-info and BaseTime deposits");

    let l1_info = FoundryTxEnvelope::decode_2718(
        &mut api.raw_transaction(hashes[0]).await.unwrap().unwrap().as_ref(),
    )
    .unwrap();
    let FoundryTxEnvelope::Deposit(l1_info) = l1_info else {
        panic!("tx[0] must be the L1-info deposit");
    };
    assert_eq!(l1_info.from, SystemAddresses::DEPOSITOR_ACCOUNT);
    assert_eq!(l1_info.to, TxKind::Call(Predeploys::L1_BLOCK_INFO));
    assert!(!l1_info.is_system_transaction);
    let info = L1BlockInfoTx::decode_calldata(&l1_info.input).unwrap();
    assert_eq!(info.id().number, block.header.number);
    assert_eq!(info.time(), block.header.timestamp);
    assert_eq!(
        provider.get_storage_at(Predeploys::L1_BLOCK_INFO, U256::ZERO).await.unwrap(),
        U256::from(block.header.number) | (U256::from(block.header.timestamp) << 64)
    );

    let base_time = FoundryTxEnvelope::decode_2718(
        &mut api.raw_transaction(hashes[1]).await.unwrap().unwrap().as_ref(),
    )
    .unwrap();
    let FoundryTxEnvelope::Deposit(base_time) = base_time else {
        panic!("tx[1] must be the BaseTime deposit");
    };
    assert_eq!(base_time.from, SystemAddresses::DEPOSITOR_ACCOUNT);
    assert_eq!(base_time.to, TxKind::Call(Predeploys::BASE_TIME));
    assert_eq!(
        base_time.encoded_2718(),
        BaseTimeUpdateTx::new(200).unwrap().into_deposit_tx(block.header.number).encoded_2718()
    );
    assert_eq!(
        base_time.source_hash,
        DepositSourceDomain::BaseTime(BaseTimeDepositSource { block_number: block.header.number })
            .source_hash()
    );

    // Block hashes are canonical deposit hashes and resolve through the transaction APIs.
    for hash in hashes {
        let raw = api.raw_transaction(hash).await.unwrap().unwrap();
        assert_eq!(keccak256(&raw), hash);
        let receipt = provider.get_transaction_receipt(hash).await.unwrap().unwrap();
        assert_eq!(receipt.transaction_hash, hash);
        assert!(receipt.status());
        assert!(provider.get_transaction_by_hash(hash).await.unwrap().is_some());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_denim_pending_state_includes_system_transactions() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let call = WithOtherFields::new(
        TransactionRequest::default()
            .with_to(Predeploys::BASE_TIME)
            .with_input(IBaseTime::timestampMillisPartCall {}.abi_encode()),
    );

    for _ in 0..4 {
        api.mine_one().await.unwrap();
    }
    let latest = provider.call(call.clone()).block(BlockId::latest()).await.unwrap();
    assert_eq!(IBaseTime::timestampMillisPartCall::abi_decode_returns(&latest).unwrap(), 800);

    // Block 5 rolls over to a new second; pending state must already reflect its deposit.
    let pending = provider.call(call.clone()).block(BlockId::pending()).await.unwrap();
    assert_eq!(IBaseTime::timestampMillisPartCall::abi_decode_returns(&pending).unwrap(), 0);

    api.mine_one().await.unwrap();
    let mined = provider.call(call).block(BlockId::latest()).await.unwrap();
    assert_eq!(mined, pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_fork_denim_preserves_l1_fee_state() {
    let source_config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (source_api, source_handle) = spawn(source_config).await;
    // Give the source chain L1 fee parameters that differ from the standalone defaults.
    for slot in 0..9u64 {
        let value = match slot {
            3 => (U256::from(1_234_567) << 96) | (U256::from(7_654) << 64) | U256::from(17),
            8 => (U256::from(650) << 96) | (U256::from(42) << 64) | U256::from(789),
            _ => U256::from(0x1234_5678_u64 + slot),
        };
        source_api
            .anvil_set_storage_at(Predeploys::L1_BLOCK_INFO, U256::from(slot), B256::from(value))
            .await
            .unwrap();
    }
    source_api.mine_one().await.unwrap();
    assert!(
        source_handle.http_provider().get_code_at(Predeploys::BASE_TIME).await.unwrap().is_empty()
    );

    let target_config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Denim.into()))
        .with_eth_rpc_url(Some(source_handle.http_endpoint()));
    let (target_api, target_handle) = spawn(target_config).await;
    let provider = target_handle.http_provider();

    let l1_block_state = async || {
        let mut slots = Vec::new();
        for slot in 0..9u64 {
            slots.push(
                provider.get_storage_at(Predeploys::L1_BLOCK_INFO, U256::from(slot)).await.unwrap(),
            );
        }
        slots
    };
    let mut expected = l1_block_state().await;
    // Continuing the same L1 origin advances its sequence and retains all fee fields.
    expected[3] += U256::ONE;
    target_api.mine_one().await.unwrap();
    assert_eq!(l1_block_state().await, expected, "fork L1Block fee state must be preserved");

    // Both protocol deposits prefix forked Denim blocks and execute successfully.
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    let hashes = block.transactions.hashes().collect::<Vec<_>>();
    assert_eq!(hashes.len(), 2);
    for (index, hash) in hashes.into_iter().enumerate() {
        let receipt = provider.get_transaction_receipt(hash).await.unwrap().unwrap();
        assert!(receipt.status());
        assert_eq!(receipt.transaction_index, Some(index as u64));
    }
    let millis_part = provider
        .call(WithOtherFields::new(
            TransactionRequest::default()
                .with_to(Predeploys::BASE_TIME)
                .with_input(IBaseTime::timestampMillisPartCall {}.abi_encode()),
        ))
        .block(BlockId::latest())
        .await
        .unwrap();
    assert_eq!(IBaseTime::timestampMillisPartCall::abi_decode_returns(&millis_part).unwrap(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_mines_deposit_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let sender_before = provider.get_balance(from).await.unwrap();
    let recipient_before = provider.get_balance(to).await.unwrap();
    let mint = 1_000;
    let value = U256::from(600);
    let envelope = FoundryTxEnvelope::Deposit(Sealed::new(TxDeposit {
        source_hash: B256::with_last_byte(1),
        from,
        to: TxKind::Call(to),
        mint,
        value,
        gas_limit: 100_000,
        is_system_transaction: false,
        input: Bytes::new(),
    }));

    let pending = provider.send_raw_transaction(&envelope.encoded_2718()).await.unwrap();
    let receipt = pending.get_receipt().await.unwrap();

    assert!(receipt.status());
    assert_eq!(provider.get_balance(from).await.unwrap(), sender_before + U256::from(mint) - value);
    assert_eq!(provider.get_balance(to).await.unwrap(), recipient_before + value);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_includes_failed_deposit_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();
    let target = Address::repeat_byte(0xcc);
    api.anvil_set_code(target, Bytes::from_static(&[0xfe])).await.unwrap();
    let sender_before = provider.get_balance(from).await.unwrap();
    let envelope = FoundryTxEnvelope::Deposit(Sealed::new(TxDeposit {
        source_hash: B256::with_last_byte(2),
        from,
        to: TxKind::Call(target),
        mint: 1_000,
        value: U256::from(600),
        gas_limit: 100_000,
        is_system_transaction: false,
        input: Bytes::new(),
    }));

    let receipt = provider
        .send_raw_transaction(&envelope.encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(!receipt.status());
    assert_eq!(provider.get_balance(from).await.unwrap(), sender_before + U256::from(1_000));
    assert_eq!(provider.get_balance(target).await.unwrap(), U256::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_call_and_estimate_are_read_only() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let request = eip8130_simulation_request(sender);
    let balance_before = provider.get_balance(sender).await.unwrap();
    let nonce_before = provider.get_transaction_count(sender).await.unwrap();

    let output = provider.call(request.clone()).await.unwrap();
    let estimate = provider.estimate_gas(request).await.unwrap();
    let access_list_error =
        provider.create_access_list(&eip8130_simulation_request(sender)).await.unwrap_err();

    assert!(output.is_empty());
    assert!(estimate > 0);
    assert!(access_list_error.to_string().contains("does not support EIP-8130"));
    assert_eq!(provider.get_balance(sender).await.unwrap(), balance_before);
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), nonce_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_estimate_includes_sponsored_payer_auth() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let wallets: Vec<_> = handle.dev_wallets().collect();
    let sender = wallets[0].address();
    let payer = wallets[1].address();
    let self_pay = eip8130_simulation_request(sender);
    let sponsored = serde_json::from_value(serde_json::json!({
        "from": sender,
        "payer": payer,
        "calls": [],
        "maxFeePerGas": "0x3b9aca00",
        "gas": "0x30d40"
    }))
    .unwrap();

    let self_pay_gas = provider.estimate_gas(self_pay).await.unwrap();
    let sponsored_gas = provider.estimate_gas(sponsored).await.unwrap();

    assert!(
        sponsored_gas > self_pay_gas,
        "self-pay estimate {self_pay_gas}, sponsored estimate {sponsored_gas}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_estimate_prices_authentication_scheme() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let k1 = provider.estimate_gas(eip8130_simulation_request(sender)).await.unwrap();
    let p256 = provider
        .estimate_gas(
            serde_json::from_value(serde_json::json!({
                "from": sender,
                "calls": [],
                "senderAuth": eip8130_auth_blob(Eip8130Contracts::P256_AUTHENTICATOR, 128),
                "maxFeePerGas": "0x3b9aca00",
                "gas": "0x30d40"
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let webauthn = provider
        .estimate_gas(
            serde_json::from_value(serde_json::json!({
                "from": sender,
                "calls": [],
                "senderAuth": eip8130_auth_blob(
                    Eip8130Contracts::WEBAUTHN_AUTHENTICATOR,
                    1024
                ),
                "maxFeePerGas": "0x3b9aca00",
                "gas": "0x30d40"
            }))
            .unwrap(),
        )
        .await
        .unwrap();

    assert!(p256 > k1, "P-256 estimate {p256} must exceed K1 {k1}");
    assert!(webauthn > p256, "WebAuthn estimate {webauthn} must exceed P-256 {p256}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_simulation_and_nonce_key_are_rejected_before_zenith() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();

    let call_error = provider.call(eip8130_simulation_request(sender)).await.unwrap_err();
    let estimate_error =
        provider.estimate_gas(eip8130_simulation_request(sender)).await.unwrap_err();
    let access_list_error =
        provider.create_access_list(&eip8130_simulation_request(sender)).await.unwrap_err();
    let nonce_error = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", U256::ONE))
        .await
        .unwrap_err();

    for error in [
        call_error.to_string(),
        estimate_error.to_string(),
        access_list_error.to_string(),
        nonce_error.to_string(),
    ] {
        assert!(error.contains("not active before the Zenith hard fork"), "{error}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_estimate_validates_sender() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let wallets: Vec<_> = handle.dev_wallets().collect();
    let sender = wallets[0].address();
    let other = wallets[1].address();
    let missing = serde_json::from_value(serde_json::json!({
        "type": "0x79",
        "calls": [],
        "maxFeePerGas": "0x3b9aca00",
        "gas": "0x30d40"
    }))
    .unwrap();
    let error = provider.estimate_gas(missing).await.unwrap_err();
    assert!(error.to_string().contains("invalid EIP-8130 simulation request"), "{error}");

    let explicit_sender = serde_json::from_value(serde_json::json!({
        "sender": sender,
        "calls": [],
        "maxFeePerGas": "0x3b9aca00",
        "gas": "0x30d40"
    }))
    .unwrap();
    assert!(provider.estimate_gas(explicit_sender).await.unwrap() > 0);

    let mismatch = serde_json::from_value(serde_json::json!({
        "from": sender,
        "sender": other,
        "calls": [],
        "maxFeePerGas": "0x3b9aca00",
        "gas": "0x30d40"
    }))
    .unwrap();
    let error = provider.estimate_gas(mismatch).await.unwrap_err();
    assert!(error.to_string().contains("invalid EIP-8130 simulation request"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_estimate_surfaces_phase_revert() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let target = Address::repeat_byte(0xee);
    api.anvil_set_code(target, Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xfd])).await.unwrap();
    let request = eip8130_simulation_request_with_call(sender, target);

    let error = provider.estimate_gas(request).await.unwrap_err();

    assert!(error.to_string().contains("revert"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_debug_trace_call_inspects_protocol_calls() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let target = address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee01");
    api.anvil_set_code(target, Bytes::from_static(&[0x60, 0x01, 0x50, 0x00])).await.unwrap();

    let trace = provider
        .debug_trace_call(
            eip8130_simulation_request_with_call(sender, target),
            BlockId::latest(),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default()),
            ),
        )
        .await
        .unwrap();

    let GethTrace::CallTracer(frame) = trace else { panic!("expected call trace") };
    assert_eq!(frame.calls.len(), 1);
    assert_eq!(frame.calls[0].to, Some(target));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_debug_trace_transaction_inspects_protocol_calls() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let target = address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee02");
    api.anvil_set_code(target, Bytes::from_static(&[0x60, 0x01, 0x50, 0x00])).await.unwrap();
    let receipt = provider
        .send_raw_transaction(
            &eip8130_envelope_with(
                &signer,
                vec![vec![Call { to: target, data: Bytes::new() }]],
                Bytes::new(),
            )
            .encoded_2718(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let trace = provider
        .debug_trace_transaction(
            receipt.transaction_hash(),
            GethDebugTracingOptions::default()
                .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                .with_call_config(CallConfig::default()),
        )
        .await
        .unwrap();

    let GethTrace::CallTracer(frame) = trace else { panic!("expected call trace") };
    assert_eq!(frame.calls.len(), 1);
    assert_eq!(frame.calls[0].to, Some(target));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_nonce_key_rpc_reads_channel_state() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let nonce_key = U256::from(7);
    let slot = NonceManagerStorage::nonce_slot(sender, nonce_key).unwrap();
    api.anvil_set_storage_at(
        NonceManagerStorage::ADDRESS,
        slot,
        B256::from(B256::with_last_byte(42).0),
    )
    .await
    .unwrap();

    let protocol = provider.get_transaction_count(sender).await.unwrap();
    let protocol_with_key = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", U256::ZERO))
        .await
        .unwrap();
    let channel = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", nonce_key))
        .await
        .unwrap();
    let max_error = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", U256::MAX))
        .await
        .unwrap_err();

    assert_eq!(protocol_with_key, U256::from(protocol));
    assert_eq!(channel, U256::from(42));
    assert!(max_error.to_string().contains("no per-channel counter"), "{max_error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_keeps_independent_nonce_channels() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let sender = signer.address();
    let first_key = U256::ONE;
    let second_key = U256::from(2);

    let first = provider
        .send_raw_transaction(&eip8130_envelope_with_nonce(&signer, first_key, 0, 0).encoded_2718())
        .await
        .unwrap();
    let second = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce(&signer, second_key, 0, 0).encoded_2718(),
        )
        .await
        .unwrap();
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 2);
    let content = provider.txpool_content().await.unwrap();
    let pending = content.pending.get(&sender).unwrap();
    assert!(pending.contains_key("1:0"));
    assert!(pending.contains_key("2:0"));

    api.mine_one().await.unwrap();
    assert!(first.get_receipt().await.unwrap().status());
    assert!(second.get_receipt().await.unwrap().status());

    for nonce_key in [first_key, second_key] {
        let nonce = provider
            .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", nonce_key))
            .await
            .unwrap();
        assert_eq!(nonce, U256::ONE);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_orders_channel_heads_by_fee() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();

    let low = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(&signer, U256::ONE, 0, 0, 1_000_000_000)
                .encoded_2718(),
        )
        .await
        .unwrap();
    let high = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(&signer, U256::from(2), 0, 0, 2_000_000_000)
                .encoded_2718(),
        )
        .await
        .unwrap();

    api.mine_one().await.unwrap();
    let high = high.get_receipt().await.unwrap();
    let low = low.get_receipt().await.unwrap();
    // System deposits occupy the prefix of a Denim block; assert the channel ordering without
    // assuming that the first user transaction is at block index zero.
    assert_eq!(low.transaction_index, high.transaction_index.map(|index| index + 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_promotes_filled_channel_gap() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let sender = signer.address();
    let nonce_key = U256::from(11);

    let sequence_one = provider
        .send_raw_transaction(&eip8130_envelope_with_nonce(&signer, nonce_key, 1, 0).encoded_2718())
        .await
        .unwrap();
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 1);

    let sequence_zero = provider
        .send_raw_transaction(&eip8130_envelope_with_nonce(&signer, nonce_key, 0, 0).encoded_2718())
        .await
        .unwrap();
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 2);
    assert_eq!(status.queued, 0);

    api.mine_one().await.unwrap();
    assert!(sequence_zero.get_receipt().await.unwrap().status());
    assert!(sequence_one.get_receipt().await.unwrap().status());
    let nonce = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", nonce_key))
        .await
        .unwrap();
    assert_eq!(nonce, U256::from(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_enforces_lane_replacement_price() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let nonce_key = U256::from(12);
    let original = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(&signer, nonce_key, 0, 0, 1_000_000_000)
                .encoded_2718(),
        )
        .await
        .unwrap();
    let original_hash = *original.tx_hash();
    let error = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(&signer, nonce_key, 0, 0, 1_050_000_000)
                .encoded_2718(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("replacement transaction underpriced"), "{error}");
    assert_eq!(provider.txpool_status().await.unwrap().pending, 1);

    let replacement = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(&signer, nonce_key, 0, 0, 2_000_000_000)
                .encoded_2718(),
        )
        .await
        .unwrap();
    let replacement_hash = *replacement.tx_hash();
    assert_ne!(original_hash, replacement_hash);
    assert_eq!(provider.txpool_status().await.unwrap().pending, 1);

    api.mine_one().await.unwrap();
    assert!(provider.get_transaction_receipt(original_hash).await.unwrap().is_none());
    assert!(replacement.get_receipt().await.unwrap().status());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_replaces_nonce_free_by_replay_id() {
    let genesis_timestamp =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Zenith.into()))
        .with_genesis_timestamp(Some(genesis_timestamp));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let now = api.anvil_node_info().await.unwrap().current_block_timestamp;
    let valid_before = (now + 10) * 1_000;
    let original = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(
                &signer,
                Eip8130Constants::NONCE_KEY_MAX,
                0,
                valid_before,
                1_000_000_000,
            )
            .encoded_2718(),
        )
        .await
        .unwrap();
    let original_hash = *original.tx_hash();
    let replacement = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(
                &signer,
                Eip8130Constants::NONCE_KEY_MAX,
                0,
                valid_before,
                2_000_000_000,
            )
            .encoded_2718(),
        )
        .await
        .unwrap();
    let independent = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(
                &signer,
                Eip8130Constants::NONCE_KEY_MAX,
                0,
                valid_before + 1,
                1_000_000_000,
            )
            .encoded_2718(),
        )
        .await
        .unwrap();
    let replacement_hash = *replacement.tx_hash();
    let independent_hash = *independent.tx_hash();
    assert_eq!(provider.txpool_status().await.unwrap().pending, 2);

    api.mine_one().await.unwrap();
    assert!(provider.get_transaction_receipt(original_hash).await.unwrap().is_none());
    assert!(provider.get_transaction_receipt(replacement_hash).await.unwrap().is_some());
    assert!(provider.get_transaction_receipt(independent_hash).await.unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_rejects_mined_nonce_free_replay_at_admission() {
    let genesis_timestamp =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Zenith.into()))
        .with_genesis_timestamp(Some(genesis_timestamp));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let valid_before = (genesis_timestamp + 20) * 1_000;
    provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(
                &signer,
                Eip8130Constants::NONCE_KEY_MAX,
                0,
                valid_before,
                1_000_000_000,
            )
            .encoded_2718(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let error = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(
                &signer,
                Eip8130Constants::NONCE_KEY_MAX,
                0,
                valid_before,
                2_000_000_000,
            )
            .encoded_2718(),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("replay"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_drops_expired_nonce_free_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let now = api.anvil_node_info().await.unwrap().current_block_timestamp;
    let valid_before = (now + 1) * 1_000;
    let pending = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce(&signer, Eip8130Constants::NONCE_KEY_MAX, 0, valid_before)
                .encoded_2718(),
        )
        .await
        .unwrap();
    let hash = *pending.tx_hash();

    api.evm_increase_time(U256::from(2)).await.unwrap();
    api.mine_one().await.unwrap();

    assert!(provider.get_transaction_receipt(hash).await.unwrap().is_none());
    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_snapshot_revert_restores_channel_nonce() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let sender = signer.address();
    let nonce_key = U256::from(13);
    let snapshot = api.evm_snapshot().await.unwrap();

    provider
        .send_raw_transaction(&eip8130_envelope_with_nonce(&signer, nonce_key, 0, 0).encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let nonce = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", nonce_key))
        .await
        .unwrap();
    assert_eq!(nonce, U256::ONE);

    assert!(api.evm_revert(snapshot).await.unwrap());
    let nonce = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", nonce_key))
        .await
        .unwrap();
    assert_eq!(nonce, U256::ZERO);

    let valid_before = (api.anvil_node_info().await.unwrap().current_block_timestamp + 100) * 1_000;
    let receipt = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce(&signer, nonce_key, 0, valid_before).encoded_2718(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_snapshot_revert_clears_pending_transactions() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let snapshot = api.evm_snapshot().await.unwrap();
    let _pending = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce(&signer, U256::from(15), 0, 0).encoded_2718(),
        )
        .await
        .unwrap();
    assert_eq!(provider.txpool_status().await.unwrap().pending, 1);

    assert!(api.evm_revert(snapshot).await.unwrap());

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_state_cheat_clears_pending_transactions() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let _pending = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce(&signer, U256::from(16), 0, 0).encoded_2718(),
        )
        .await
        .unwrap();
    assert_eq!(provider.txpool_status().await.unwrap().pending, 1);

    api.anvil_set_balance(signer.address(), U256::ONE).await.unwrap();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_mines_eip8130_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let sender = signer.address();
    let before = provider.get_balance(sender).await.unwrap();
    assert_eq!(provider.get_code_at(NONCE_MANAGER).await.unwrap().as_ref(), &[0xef]);
    let envelope = eip8130_envelope(&signer);

    let receipt = provider
        .send_raw_transaction(&envelope.encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());
    let receipt_json = serde_json::to_value(&receipt).unwrap();
    assert!(receipt_json.get("phaseStatuses").is_none(), "{receipt_json}");
    assert_eq!(receipt_json["payer"], serde_json::to_value(sender).unwrap());
    let mined = provider.get_transaction_by_hash(receipt.tx_hash()).await.unwrap().unwrap();
    assert_eq!(mined.ty(), 0x79);
    let mined_json = serde_json::to_value(mined).unwrap();
    assert_eq!(mined_json["tx"]["nonceKey"], "0x0", "{mined_json}");
    assert!(mined_json["tx"].get("calls").is_some());
    assert!(mined_json.get("senderAuth").is_some());
    assert!(provider.get_balance(sender).await.unwrap() < before);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_rejects_protocol_nonce_replay_at_admission() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    provider
        .send_raw_transaction(&eip8130_envelope(&signer).encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let error = provider
        .send_raw_transaction(
            &eip8130_envelope_with(&signer, Vec::new(), Bytes::from_static(&[0x01])).encoded_2718(),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("below the channel nonce"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_rejects_invalid_auth_at_admission() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();

    let error = provider
        .send_raw_transaction(&malformed_configured_eip8130_envelope(&signer).encoded_2718())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("EIP-8130 transaction rejected"), "{error}");
    assert_eq!(provider.txpool_status().await.unwrap().pending, 0);

    let envelope = malformed_configured_eip8130_envelope_with_nonce(&signer, U256::ONE, 1);
    let error = provider.send_raw_transaction(&envelope.encoded_2718()).await.unwrap_err();

    assert!(error.to_string().contains("EIP-8130 transaction rejected"), "{error}");
    assert_eq!(provider.txpool_status().await.unwrap().queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_receipt_reports_phase_statuses_and_metadata() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let sender = signer.address();
    let target = Address::repeat_byte(0xdd);
    api.anvil_set_code(target, Bytes::from_static(&[0x00])).await.unwrap();
    let envelope = eip8130_envelope_with(
        &signer,
        vec![vec![Call { to: target, data: Bytes::new() }]],
        Bytes::from_static(&[0xaa]),
    );

    let receipt = provider
        .send_raw_transaction(&envelope.encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let value = serde_json::to_value(receipt).unwrap();

    assert_eq!(value["phaseStatuses"], serde_json::json!(["0x1"]));
    assert_eq!(value["payer"], serde_json::to_value(sender).unwrap());
    assert_eq!(value["metadata"], "0xaa");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_sponsored_receipt_reports_payer() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = PrivateKeySigner::from_bytes(&B256::with_last_byte(0x42)).unwrap();
    let payer = handle.dev_wallets().next().unwrap().clone();
    assert_eq!(provider.get_balance(sender.address()).await.unwrap(), U256::ZERO);
    let payer_before = provider.get_balance(payer.address()).await.unwrap();

    let receipt = provider
        .send_raw_transaction(&sponsored_eip8130_envelope(&sender, &payer).encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status());
    let value = serde_json::to_value(receipt).unwrap();

    assert_eq!(value["payer"], serde_json::to_value(payer.address()).unwrap());
    assert_eq!(provider.get_balance(sender.address()).await.unwrap(), U256::ZERO);
    assert!(provider.get_balance(payer.address()).await.unwrap() < payer_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_sponsored_tx_rejects_unfunded_payer() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().clone();
    let payer = PrivateKeySigner::from_bytes(&B256::with_last_byte(0x43)).unwrap();
    assert_eq!(provider.get_balance(payer.address()).await.unwrap(), U256::ZERO);

    let error = provider
        .send_raw_transaction(&sponsored_eip8130_envelope(&sender, &payer).encoded_2718())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("gas payer balance"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_reserves_pending_payer_balance() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let sender = PrivateKeySigner::from_bytes(&B256::with_last_byte(0x44)).unwrap();
    let payer = handle.dev_wallets().next().unwrap().clone();
    api.anvil_set_balance(payer.address(), U256::from(300_000_000_000_000u64)).await.unwrap();
    let _first = provider
        .send_raw_transaction(
            &sponsored_eip8130_envelope_with_nonce(&sender, &payer, U256::from(30), 0)
                .encoded_2718(),
        )
        .await
        .unwrap();

    let error = provider
        .send_raw_transaction(
            &sponsored_eip8130_envelope_with_nonce(&sender, &payer, U256::from(31), 0)
                .encoded_2718(),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("pending reservation"), "{error}");
    assert_eq!(provider.txpool_status().await.unwrap().pending, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_receipt_reports_partial_phase_revert() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let success = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1");
    let reverter = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa2");
    api.anvil_set_code(success, Bytes::from_static(&[0x00])).await.unwrap();
    api.anvil_set_code(reverter, Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xfd]))
        .await
        .unwrap();
    let envelope = eip8130_envelope_with(
        &signer,
        vec![
            vec![Call { to: success, data: Bytes::new() }],
            vec![Call { to: reverter, data: Bytes::new() }],
        ],
        Bytes::new(),
    );

    let receipt = provider
        .send_raw_transaction(&envelope.encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let value = serde_json::to_value(receipt).unwrap();

    assert_eq!(value["status"], "0x0");
    assert_eq!(value["phaseStatuses"], serde_json::json!(["0x1", "0x0"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_same_block_receipts_keep_phase_statuses_isolated() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let success = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa3");
    let reverter = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa4");
    api.anvil_set_code(success, Bytes::from_static(&[0x00])).await.unwrap();
    api.anvil_set_code(reverter, Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xfd]))
        .await
        .unwrap();
    let first = provider
        .send_raw_transaction(
            &eip8130_envelope_with_channel_calls(
                &signer,
                U256::from(20),
                vec![vec![Call { to: success, data: Bytes::new() }]],
            )
            .encoded_2718(),
        )
        .await
        .unwrap();
    let second = provider
        .send_raw_transaction(
            &eip8130_envelope_with_channel_calls(
                &signer,
                U256::from(21),
                vec![
                    vec![Call { to: success, data: Bytes::new() }],
                    vec![Call { to: reverter, data: Bytes::new() }],
                ],
            )
            .encoded_2718(),
        )
        .await
        .unwrap();

    api.mine_one().await.unwrap();
    let first = serde_json::to_value(first.get_receipt().await.unwrap()).unwrap();
    let second = serde_json::to_value(second.get_receipt().await.unwrap()).unwrap();

    assert_eq!(first["phaseStatuses"], serde_json::json!(["0x1"]));
    assert_eq!(second["phaseStatuses"], serde_json::json!(["0x1", "0x0"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_beryl_rejects_eip8130_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (_api, handle) = spawn(config).await;
    let signer = handle.dev_wallets().next().unwrap().clone();

    let err = handle
        .http_provider()
        .send_raw_transaction(&eip8130_envelope(&signer).encoded_2718())
        .await
        .unwrap_err();
    let nonce_error = handle
        .http_provider()
        .raw_request::<_, U256>(
            "eth_getTransactionCount".into(),
            (signer.address(), "latest", U256::ONE),
        )
        .await
        .unwrap_err();

    assert!(err.to_string().contains("gated behind Zenith"), "{err}");
    assert!(
        nonce_error.to_string().contains("not active before the Zenith hard fork"),
        "{nonce_error}"
    );
}

#[cfg(feature = "optimism")]
#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_is_rejected_by_non_base_networks() {
    let configs =
        [NodeConfig::test(), NodeConfig::test().with_optimism(), NodeConfig::test_tempo()];

    for config in configs {
        let (_api, handle) = spawn(config).await;
        let signer = handle.dev_wallets().next().unwrap().clone();
        let error = handle
            .http_provider()
            .send_raw_transaction(&eip8130_envelope(&signer).encoded_2718())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("gated behind Zenith"), "{error}");
    }
}

async fn base_time_ms(provider: &RetryProvider, block: BlockId) -> u64 {
    let result = provider
        .call(WithOtherFields::new(
            TransactionRequest::default()
                .with_to(Predeploys::BASE_TIME)
                .with_input(IBaseTime::timestampMsCall {}.abi_encode()),
        ))
        .block(block)
        .await
        .unwrap();
    IBaseTime::timestampMsCall::abi_decode_returns(&result).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn base_denim_interval_cadence() {
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Denim.into()))
        .with_genesis_timestamp(Some(1_000u64))
        .with_genesis_block_number(Some(17u64))
        .with_blocktime(Some(Duration::from_millis(200)));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    tokio::time::timeout(Duration::from_secs(10), async {
        while provider.get_block_number().await.unwrap() < 27 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    for number in 17..=27u64 {
        assert_eq!(base_time_ms(&provider, number.into()).await, 1_000_000 + (number - 17) * 200);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_fork_denim_continues_every_parent_phase() {
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Denim.into()))
        .with_genesis_block_number(Some(2u64));
    let (source_api, source_handle) = spawn(config).await;
    let admin = B256::from(U256::from(0x1234));
    source_api
        .anvil_set_storage_at(Predeploys::BASE_TIME, BaseTime::ADMIN_SLOT, admin)
        .await
        .unwrap();
    let source_provider = source_handle.http_provider();
    for _ in 0..5 {
        let before = base_time_ms(&source_provider, BlockId::latest()).await;
        let config = NodeConfig::test_base()
            .with_hardfork(Some(BaseUpgrade::Denim.into()))
            .with_eth_rpc_url(Some(source_handle.http_endpoint()));
        let (api, handle) = spawn(config).await;
        let provider = handle.http_provider();
        assert_eq!(
            provider.get_storage_at(Predeploys::BASE_TIME, BaseTime::ADMIN_SLOT).await.unwrap(),
            U256::from_be_bytes(admin.0)
        );
        assert_eq!(base_time_ms(&provider, BlockId::pending()).await, before + 200);
        api.mine_one().await.unwrap();
        assert_eq!(base_time_ms(&provider, BlockId::latest()).await, before + 200);
        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: None })).await.unwrap();
        assert_eq!(base_time_ms(&provider, BlockId::pending()).await, before + 200);
        api.mine_one().await.unwrap();
        assert_eq!(base_time_ms(&provider, BlockId::latest()).await, before + 200);
        source_api.mine_one().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_denim_time_controls_and_state_restore() {
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Denim.into()))
        .with_genesis_timestamp(Some(1_000u64));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    for _ in 0..4 {
        api.mine_one().await.unwrap();
    }
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 1_000_800);
    api.evm_increase_time(U256::from(10)).await.unwrap();
    let snapshot = api.evm_snapshot().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::pending()).await, 1_011_000);
    assert_eq!(base_time_ms(&provider, BlockId::pending()).await, 1_011_000);
    api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 1_011_000);
    api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 1_011_200);
    assert!(api.evm_revert(snapshot).await.unwrap());
    api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 1_011_000);

    api.evm_set_next_block_timestamp(2_000).unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::pending()).await, 2_000_200);
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 2_000_400);
    api.evm_set_block_timestamp_interval(2).unwrap();
    api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 2_002_600);
    api.evm_remove_block_timestamp_interval().unwrap();
    api.evm_set_time(3_000).unwrap();
    api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 3_000_800);

    let dump = api.anvil_dump_state(Some(true)).await.unwrap();
    let (loaded_api, loaded_handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()))).await;
    loaded_api.anvil_load_state(dump).await.unwrap();
    loaded_api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&loaded_handle.http_provider(), BlockId::latest()).await, 3_001_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_denim_failed_deposit_preserves_state_and_time_override() {
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Denim.into()))
        .with_genesis_timestamp(Some(1_000u64));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let code = provider.get_code_at(Predeploys::BASE_TIME).await.unwrap();
    api.anvil_set_code(Predeploys::BASE_TIME, Bytes::from_static(&[0x5f, 0x5f, 0xfd]))
        .await
        .unwrap();
    api.evm_set_next_block_timestamp(2_000).unwrap();
    let error = api.mine_one().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Internal error: \"required Base system deposit at transaction index 1 failed\""
    );
    assert_eq!(provider.get_block_number().await.unwrap(), 0);
    assert_eq!(
        provider.get_storage_at(Predeploys::L1_BLOCK_INFO, U256::ZERO).await.unwrap(),
        U256::ZERO
    );
    api.anvil_set_code(Predeploys::BASE_TIME, code).await.unwrap();
    api.mine_one().await.unwrap();
    assert_eq!(base_time_ms(&provider, BlockId::latest()).await, 2_000_200);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_denim_system_deposits_do_not_retry_unfittable_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()));
    let (api, handle) = spawn(config).await;
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumWallet = accounts[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    // The system deposits consume block gas first, so this transaction cannot fit in a Denim
    // block. Their inclusion must not count as pool progress and retrigger automine.
    let pending = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(accounts[0].address())
                .with_to(accounts[1].address())
                .with_value(U256::from(1))
                .with_gas_limit(api.gas_limit().to())
                .into(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while provider.get_block_number().await.unwrap() == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    // A retry loop would keep mining deposit-only blocks during this window.
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(provider.get_block_number().await.unwrap(), 1);
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 2);
    assert!(provider.get_transaction_receipt(*pending.tx_hash()).await.unwrap().is_none());
    assert_eq!(provider.txpool_status().await.unwrap().pending, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_denim_memory_reset_reinstalls_base_time() {
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Denim.into()))
        .with_genesis_timestamp(Some(1_000u64));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    api.mine_one().await.unwrap();

    api.anvil_reset(None).await.unwrap();
    for expected in [1_000_200, 1_000_400, 1_000_600, 1_000_800, 1_001_000] {
        api.mine_one().await.unwrap();
        assert_eq!(base_time_ms(&provider, BlockId::latest()).await, expected);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_denim_load_pre_denim_state_installs_base_time() {
    let (source, _source_handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()))).await;
    source.mine_one().await.unwrap();
    let dump = source.anvil_dump_state(Some(true)).await.unwrap();

    let (api, handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()))).await;
    let provider = handle.http_provider();
    api.anvil_load_state(dump).await.unwrap();
    let parent = provider.get_block(BlockId::latest()).await.unwrap().unwrap().header.timestamp;
    for (index, millis) in [200, 400, 600, 800, 1_000].into_iter().enumerate() {
        api.mine_one().await.unwrap();
        assert_eq!(
            base_time_ms(&provider, BlockId::latest()).await,
            parent * 1_000 + millis,
            "block {index} after loading a pre-Denim dump"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_denim_rejected_state_load_is_atomic() {
    let config = || NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()));
    let sentinel = Address::repeat_byte(0x77);

    let (source, _source_handle) = spawn(config()).await;
    source.anvil_set_balance(sentinel, U256::from(123)).await.unwrap();
    source.mine_one().await.unwrap();
    source.mine_one().await.unwrap();
    let mut invalid_state = source.backend.serialized_state(false).await.unwrap();
    let base_time = invalid_state.accounts.get_mut(&Predeploys::BASE_TIME).unwrap();
    base_time
        .storage
        .insert(B256::from(BaseTime::IMPLEMENTATION_SLOT.to_be_bytes::<32>()), B256::ZERO);
    base_time.storage.insert(
        B256::from(BaseTime::ADMIN_SLOT.to_be_bytes::<32>()),
        B256::from(U256::from(0xdead).to_be_bytes::<32>()),
    );

    let (target, target_handle) = spawn(config()).await;
    target.anvil_set_balance(sentinel, U256::from(7)).await.unwrap();
    target.mine_one().await.unwrap();
    let original_hash = target.backend.best_hash();
    let original_number = target.backend.best_number();

    target.backend.load_state(invalid_state).await.unwrap_err();
    assert_eq!(target.backend.best_hash(), original_hash);
    assert_eq!(target.backend.best_number(), original_number);
    assert_eq!(target.backend.current_balance(sentinel).await.unwrap(), U256::from(7));

    target.mine_one().await.unwrap();
    assert_eq!(target.backend.best_number(), original_number + 1);
    assert_eq!(base_time_ms(&target_handle.http_provider(), BlockId::latest()).await % 1_000, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_inferred_denim_fork_resets_to_pre_denim() {
    let (denim, denim_handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()))).await;
    denim.mine_one().await.unwrap();
    let (beryl, beryl_handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()))).await;
    beryl.mine_one().await.unwrap();

    let (api, handle) =
        spawn(NodeConfig::test_base().with_eth_rpc_url(Some(denim_handle.http_endpoint()))).await;
    assert_eq!(api.backend.hardfork(), BaseUpgrade::Denim.into());

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(beryl_handle.http_endpoint()),
        block_number: None,
    }))
    .await
    .unwrap();
    assert_eq!(api.backend.hardfork(), BaseUpgrade::Beryl.into());
    assert!(handle.http_provider().get_code_at(Predeploys::BASE_TIME).await.unwrap().is_empty());

    api.mine_one().await.unwrap();
    let block = handle.http_provider().get_block(BlockId::latest()).await.unwrap().unwrap();
    assert!(block.transactions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_fork_denim_rejects_pre_jovian_l1_block() {
    let (source, source_handle) =
        spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()))).await;
    let denim_fork = |block: u64| {
        NodeConfig::test_base()
            .with_hardfork(Some(BaseUpgrade::Denim.into()))
            .with_eth_rpc_url(Some(source_handle.http_endpoint()))
            .with_fork_block_number(Some(block))
            .with_no_storage_caching(true)
    };
    source.mine_one().await.unwrap();
    let (api, _handle) = spawn(denim_fork(1)).await;

    // From block 2, L1Block stands in for an implementation without the Jovian setter.
    source
        .anvil_set_code(Predeploys::L1_BLOCK_INFO, Bytes::from_static(&[0x5f, 0x5f, 0xfd]))
        .await
        .unwrap();
    source.mine_one().await.unwrap();

    let Err(error) = anvil::try_spawn(denim_fork(2)).await else {
        panic!("a Denim fork whose L1Block rejects the L1-info deposit must not start");
    };
    assert_eq!(
        format!("{error:#}"),
        "failed to create genesis: failed to process AnyRequest: Denim system deposits fail on \
         this fork; its L1Block likely predates Jovian (fork at or after the Jovian upgrade, or \
         use an earlier --hardfork): Internal error: \"required Base system deposit at transaction \
         index 0 failed\""
    );

    // Resetting a running node onto that block is rejected and leaves it mining on block 1.
    let error = api
        .anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(2) }))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "failed to process AnyRequest: Denim system deposits fail on this fork; its L1Block \
         likely predates Jovian (fork at or after the Jovian upgrade, or use an earlier \
         --hardfork): Internal error: \"required Base system deposit at transaction index 0 \
         failed\""
    );
    api.mine_one().await.unwrap();
    assert_eq!(api.block_number().unwrap(), U256::from(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_denim_pending_receipts_include_system_transactions() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Denim.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    let block = provider.get_block(BlockId::pending()).await.unwrap().unwrap();
    let receipts = provider.get_block_receipts(BlockId::pending()).await.unwrap().unwrap();
    assert_eq!(receipts.len(), 2, "the pending block always contains the system deposits");
    assert_eq!(
        receipts.iter().map(|receipt| receipt.transaction_hash).collect::<Vec<_>>(),
        block.transactions.hashes().collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn base_reset_restores_genesis_state() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Zenith.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let read_state = async || {
        let mut slots = Vec::new();
        for slot in [1u64, 3, 7] {
            slots.push(
                provider.get_storage_at(Predeploys::L1_BLOCK_INFO, U256::from(slot)).await.unwrap(),
            );
        }
        let mut code_hashes = Vec::new();
        for address in [BASE_CREATE2_DEPLOYER, NONCE_MANAGER, ACTIVATION_REGISTRY] {
            code_hashes.push(keccak256(provider.get_code_at(address).await.unwrap()));
        }
        (slots, code_hashes)
    };

    let genesis = read_state().await;
    assert_eq!(
        genesis,
        (
            vec![U256::from(1_000_000_000u64), U256::from(1_000_000u64) << 96, U256::ONE],
            vec![
                b256!("0xb0550b5b431e30d38000efb7107aaa0ade03d48a7198a140edda9d27134468b2"),
                keccak256([0xef]),
                keccak256([0xef]),
            ],
        )
    );

    api.anvil_reset(None).await.unwrap();

    assert_eq!(read_state().await, genesis);
}
