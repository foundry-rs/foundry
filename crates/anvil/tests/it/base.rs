//! Tests for Base chain support.

use crate::utils::http_provider_with_signer;
use alloy_consensus::{Sealed, Typed2718};
use alloy_eips::Encodable2718;
use alloy_network::{EthereumWallet, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, address, b256, keccak256};
use alloy_provider::{
    Provider,
    ext::{DebugApi, TxPoolApi},
};
use alloy_rpc_types::{
    BlockId, TransactionRequest,
    trace::geth::{
        CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType, GethDebugTracingCallOptions,
        GethDebugTracingOptions, GethTrace,
    },
};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use anvil::{NodeConfig, spawn};
use base_common_consensus::{
    Call, Eip8130Constants, Eip8130Contracts, Eip8130Signed, Predeploys, TxEip8130,
};
use base_common_precompiles::NonceManagerStorage;
use foundry_config::Config;
use foundry_evm::{hardforks::BaseUpgrade, opts::EvmOpts};
use foundry_evm_networks::NetworkConfigs;
use foundry_primitives::FoundryTxEnvelope;
use op_alloy_consensus::TxDeposit;

const ACTIVATION_REGISTRY: Address = address!("8453000000000000000000000000000000000001");
const MAINNET_BERYL_ACTIVATION_ADMIN: Address =
    address!("ce3a3bee7e72e2a24079f3c0cb3b97740ed425a9");
const NONCE_MANAGER: Address = address!("813000000000000000000000000000000000aa01");

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
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(
        tx,
        signature.as_bytes().to_vec().into(),
        Bytes::new(),
    ))
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
    let bare_auth =
        signer.sign_hash_sync(&tx.sender_signature_hash()).unwrap().as_bytes().to_vec().into();
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
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(
        tx,
        signature.as_bytes().to_vec().into(),
        Bytes::new(),
    ))
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
    FoundryTxEnvelope::Eip8130(Eip8130Signed::new(
        tx,
        signature.as_bytes().to_vec().into(),
        Bytes::new(),
    ))
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
        source_hash: b256!("0000000000000000000000000000000000000000000000000000000000000001"),
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
    let target = address!("cccccccccccccccccccccccccccccccccccccccc");
    api.anvil_set_code(target, Bytes::from_static(&[0xfe])).await.unwrap();
    let sender_before = provider.get_balance(from).await.unwrap();
    let envelope = FoundryTxEnvelope::Deposit(Sealed::new(TxDeposit {
        source_hash: b256!("0000000000000000000000000000000000000000000000000000000000000002"),
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
async fn base_eip8130_estimate_is_rejected_before_cobalt() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (_api, handle) = spawn(config).await;
    let sender = handle.dev_wallets().next().unwrap().address();

    let error =
        handle.http_provider().estimate_gas(eip8130_simulation_request(sender)).await.unwrap_err();

    assert!(error.to_string().contains("not active before the Cobalt hard fork"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_call_and_nonce_key_are_rejected_before_cobalt() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();

    let call_error = provider.call(eip8130_simulation_request(sender)).await.unwrap_err();
    let access_list_error =
        provider.create_access_list(&eip8130_simulation_request(sender)).await.unwrap_err();
    let nonce_error = provider
        .raw_request::<_, U256>("eth_getTransactionCount".into(), (sender, "latest", U256::ONE))
        .await
        .unwrap_err();

    for error in [call_error.to_string(), access_list_error.to_string(), nonce_error.to_string()] {
        assert!(error.contains("not active before the Cobalt hard fork"), "{error}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_estimate_rejects_missing_sender() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (_api, handle) = spawn(config).await;
    let request = serde_json::from_value(serde_json::json!({
        "calls": [],
        "maxFeePerGas": "0x3b9aca00",
        "gas": "0x30d40"
    }))
    .unwrap();

    let error = handle.http_provider().estimate_gas(request).await.unwrap_err();

    assert!(error.to_string().contains("invalid EIP-8130 simulation request"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_estimate_accepts_sender_and_rejects_sender_mismatch() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let wallets: Vec<_> = handle.dev_wallets().collect();
    let sender = wallets[0].address();
    let other = wallets[1].address();
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let target = address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    api.anvil_set_code(target, Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xfd])).await.unwrap();
    let request = eip8130_simulation_request_with_call(sender, target);

    let error = provider.estimate_gas(request).await.unwrap_err();

    assert!(error.to_string().contains("revert"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_debug_trace_call_inspects_protocol_calls() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = handle.dev_wallets().next().unwrap().address();
    let nonce_key = U256::from(7);
    let slot = NonceManagerStorage::nonce_slot(sender, nonce_key).unwrap();
    api.anvil_set_storage_at(
        NonceManagerStorage::ADDRESS,
        slot,
        B256::from(U256::from(42).to_be_bytes::<32>()),
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let sender = signer.address();
    let first_key = U256::from(1);
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    assert_eq!(high.transaction_index, Some(0));
    assert_eq!(low.transaction_index, Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_promotes_filled_channel_gap() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
async fn base_eip8130_txpool_replaces_with_higher_fee_in_lane() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
async fn base_eip8130_txpool_rejects_underpriced_lane_replacement() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (api, handle) = spawn(config).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let nonce_key = U256::from(14);
    let _original = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(&signer, nonce_key, 0, 0, 1_000_000_000)
                .encoded_2718(),
        )
        .await
        .unwrap();

    let error = provider
        .send_raw_transaction(
            &eip8130_envelope_with_nonce_and_fee(&signer, nonce_key, 0, 0, 1_050_000_000)
                .encoded_2718(),
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("replacement transaction underpriced"), "{error}");
    assert_eq!(provider.txpool_status().await.unwrap().pending, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_txpool_replaces_nonce_free_by_replay_id() {
    let genesis_timestamp =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let config = NodeConfig::test_base()
        .with_hardfork(Some(BaseUpgrade::Cobalt.into()))
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
        .with_hardfork(Some(BaseUpgrade::Cobalt.into()))
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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

    api.anvil_set_balance(signer.address(), U256::from(1)).await.unwrap();

    let status = provider.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_standalone_mines_eip8130_transaction() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let mined =
        provider.get_transaction_by_hash(receipt.transaction_hash()).await.unwrap().unwrap();
    assert_eq!(mined.ty(), 0x79);
    let mined_json = serde_json::to_value(mined).unwrap();
    assert_eq!(mined_json["tx"]["nonceKey"], "0x0", "{mined_json}");
    assert!(mined_json["tx"].get("calls").is_some());
    assert!(mined_json.get("senderAuth").is_some());
    assert!(provider.get_balance(sender).await.unwrap() < before);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_rejects_protocol_nonce_replay_at_admission() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
async fn base_eip8130_rejects_invalid_configured_auth_at_admission() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();

    let error = provider
        .send_raw_transaction(&malformed_configured_eip8130_envelope(&signer).encoded_2718())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("EIP-8130 transaction rejected"), "{error}");
    assert_eq!(provider.txpool_status().await.unwrap().pending, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_rejects_invalid_buffered_auth_at_admission() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let envelope = malformed_configured_eip8130_envelope_with_nonce(&signer, U256::ONE, 1);

    let error = provider.send_raw_transaction(&envelope.encoded_2718()).await.unwrap_err();

    assert!(error.to_string().contains("EIP-8130 transaction rejected"), "{error}");
    assert_eq!(provider.txpool_status().await.unwrap().queued, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_receipt_reports_phase_statuses_and_metadata() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let signer = handle.dev_wallets().next().unwrap().clone();
    let sender = signer.address();
    let target = address!("dddddddddddddddddddddddddddddddddddddddd");
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let wallets: Vec<_> = handle.dev_wallets().collect();
    let sender = &wallets[0];
    let payer = &wallets[1];
    let sender_before = provider.get_balance(sender.address()).await.unwrap();
    let payer_before = provider.get_balance(payer.address()).await.unwrap();

    let receipt = provider
        .send_raw_transaction(&sponsored_eip8130_envelope(sender, payer).encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let value = serde_json::to_value(receipt).unwrap();

    assert_eq!(value["payer"], serde_json::to_value(payer.address()).unwrap());
    assert_eq!(provider.get_balance(sender.address()).await.unwrap(), sender_before);
    assert!(provider.get_balance(payer.address()).await.unwrap() < payer_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_sponsored_tx_accepts_unfunded_sender() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let sender = PrivateKeySigner::from_bytes(&B256::with_last_byte(0x42)).unwrap();
    let payer = handle.dev_wallets().next().unwrap().clone();
    let payer_before = provider.get_balance(payer.address()).await.unwrap();
    assert_eq!(provider.get_balance(sender.address()).await.unwrap(), U256::ZERO);

    let receipt = provider
        .send_raw_transaction(&sponsored_eip8130_envelope(&sender, &payer).encoded_2718())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert!(receipt.status());
    assert_eq!(provider.get_balance(sender.address()).await.unwrap(), U256::ZERO);
    assert!(provider.get_balance(payer.address()).await.unwrap() < payer_before);
}

#[tokio::test(flavor = "multi_thread")]
async fn base_eip8130_sponsored_tx_rejects_unfunded_payer() {
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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
    let config = NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Cobalt.into()));
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

    assert!(err.to_string().contains("gated behind Cobalt"), "{err}");
    assert!(
        nonce_error.to_string().contains("not active before the Cobalt hard fork"),
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

        assert!(error.to_string().contains("gated behind Cobalt"), "{error}");
    }
}
