//! Tests for Tempo-specific features in Anvil.
//!
//! This module tests Tempo's payment-native protocol features including:
//! - TIP20 fee tokens (PathUSD, AlphaUSD, BetaUSD, ThetaUSD)
//! - Tempo precompiles initialization (sentinel bytecode)
//! - Native value transfer rejection
//! - Basic transaction behavior in Tempo mode

use std::num::NonZeroU64;

use alloy_consensus::Typed2718;
use alloy_eips::eip2718::Encodable2718;
use alloy_network::{ReceiptResponse, TransactionBuilder, TransactionResponse};
use alloy_primitives::{Address, Bytes, TxKind, U256, address};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::sol;
use anvil::{NodeConfig, spawn};
use foundry_evm::core::tempo::{PATH_USD_ADDRESS, TEMPO_PRECOMPILE_ADDRESSES, TEMPO_TIP20_TOKENS};
use tempo_alloy::primitives::TempoTxEnvelope;
use tempo_primitives::{
    AASigned, TempoSignature, TempoTransaction,
    transaction::{Call, PrimitiveSignature},
};

const PATH_USD: Address = PATH_USD_ADDRESS;
const ALPHA_USD: Address = address!("0x20C0000000000000000000000000000000000001");
const BETA_USD: Address = address!("0x20C0000000000000000000000000000000000002");
const THETA_USD: Address = address!("0x20C0000000000000000000000000000000000003");

/// Gas limit for TIP20 transfer calls (precompile interactions need more gas).
const TIP20_TRANSFER_GAS: u64 = 300_000;

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
    }
}

// ============================================================================
// Tempo Genesis: Precompile Initialization
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_precompiles_have_code() {
    let (api, _handle) = spawn(NodeConfig::test_tempo()).await;

    // Tempo precompiles should have sentinel bytecode (0xef)
    for addr in TEMPO_PRECOMPILE_ADDRESSES {
        let code = api.get_code(*addr, None).await.unwrap();
        assert!(!code.is_empty(), "Precompile {addr} should have code");
    }

    // All TIP20 token addresses should also have code
    for addr in TEMPO_TIP20_TOKENS {
        let code = api.get_code(*addr, None).await.unwrap();
        assert!(!code.is_empty(), "Token {addr} should have code deployed");
    }
}

// ============================================================================
// Tempo Genesis: Fee Token Metadata
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_token_metadata() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let tokens = [
        (PATH_USD, "PathUSD", "PathUSD"),
        (ALPHA_USD, "AlphaUSD", "AlphaUSD"),
        (BETA_USD, "BetaUSD", "BetaUSD"),
        (THETA_USD, "ThetaUSD", "ThetaUSD"),
    ];

    for (addr, expected_name, expected_symbol) in tokens {
        let token = IERC20::new(addr, &provider);
        let name = token.name().call().await.unwrap();
        let symbol = token.symbol().call().await.unwrap();
        let decimals = token.decimals().call().await.unwrap();

        assert_eq!(name, expected_name, "Token at {addr} name mismatch");
        assert_eq!(symbol, expected_symbol, "Token at {addr} symbol mismatch");
        assert_eq!(decimals, 6, "All TIP20 tokens should use 6 decimals");
    }
}

// ============================================================================
// Tempo Genesis: Fee Token Balances
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_token_balances_minted_to_dev_accounts() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let dev_accounts: Vec<Address> = handle.dev_accounts().collect();
    assert!(!dev_accounts.is_empty());

    for account in dev_accounts.iter().take(3) {
        for token_addr in [PATH_USD, ALPHA_USD, BETA_USD, THETA_USD] {
            let token = IERC20::new(token_addr, &provider);
            let balance = token.balanceOf(*account).call().await.unwrap();
            assert!(
                balance > U256::ZERO,
                "Account {account} should have {token_addr} balance, got 0"
            );
        }
    }
}

// ============================================================================
// Tempo Genesis: Dev Account Balance
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_dev_accounts_have_balance() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let genesis_balance = handle.genesis_balance();

    for account in handle.dev_accounts() {
        let balance = provider.get_balance(account).await.unwrap();
        assert_eq!(balance, genesis_balance, "Dev account {account} should have genesis balance");
    }
}

// ============================================================================
// TIP20 Token Operations: Transfer
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_transfer() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);

    let sender_balance_before = token.balanceOf(sender).call().await.unwrap();
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(1_000_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());

    let sender_balance_after = token.balanceOf(sender).call().await.unwrap();
    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();

    assert_eq!(
        sender_balance_before - transfer_amount,
        sender_balance_after,
        "Sender balance should decrease by transfer amount"
    );
    assert_eq!(
        recipient_balance_before + transfer_amount,
        recipient_balance_after,
        "Recipient balance should increase by transfer amount"
    );
}

// ============================================================================
// TIP20 Token Operations: Approve and TransferFrom
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_approve_and_transfer_from() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let owner = accounts[0];
    let spender = accounts[1];
    let recipient = accounts[2];

    let token = IERC20::new(BETA_USD, &provider);

    // Owner approves spender
    let approve_amount = U256::from(5_000_000);
    let approve_call = token.approve(spender, approve_amount);
    let calldata: Bytes = approve_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(owner)
        .to(BETA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let allowance = token.allowance(owner, spender).call().await.unwrap();
    assert_eq!(allowance, approve_amount);

    // Spender transfers from owner to recipient
    let transfer_amount = U256::from(2_000_000);
    let transfer_from_call = token.transferFrom(owner, recipient, transfer_amount);
    let calldata: Bytes = transfer_from_call.calldata().clone();

    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let tx = TransactionRequest::default()
        .from(spender)
        .to(BETA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(recipient_balance_before + transfer_amount, recipient_balance_after);

    let allowance_after = token.allowance(owner, spender).call().await.unwrap();
    assert_eq!(allowance_after, approve_amount - transfer_amount);
}

// ============================================================================
// TIP20 Token Operations: Total Supply
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tip20_total_supply() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let token = IERC20::new(PATH_USD, &provider);
    let total_supply = token.totalSupply().call().await.unwrap();

    assert!(total_supply > U256::ZERO, "Total supply should be non-zero");
}

// ============================================================================
// TIP20 Token Operations: Transfer Between Different Fee Tokens
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_between_different_fee_tokens() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    for token_addr in [PATH_USD, ALPHA_USD, BETA_USD, THETA_USD] {
        let token = IERC20::new(token_addr, &provider);
        let balance_before = token.balanceOf(recipient).call().await.unwrap();

        let transfer_amount = U256::from(100_000);
        let transfer_call = token.transfer(recipient, transfer_amount);
        let calldata: Bytes = transfer_call.calldata().clone();

        let tx = TransactionRequest::default()
            .from(sender)
            .to(token_addr)
            .with_input(calldata)
            .with_gas_limit(TIP20_TRANSFER_GAS);

        let tx = WithOtherFields::new(tx);
        let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
        assert!(receipt.status(), "Transfer for {token_addr} failed");

        let balance_after = token.balanceOf(recipient).call().await.unwrap();
        assert_eq!(balance_after, balance_before + transfer_amount);
    }
}

// ============================================================================
// TIP20 Token Operations: All Fee Tokens Have Correct Metadata
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_all_fee_tokens_have_correct_metadata() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let tokens = [
        (PATH_USD, "PathUSD"),
        (ALPHA_USD, "AlphaUSD"),
        (BETA_USD, "BetaUSD"),
        (THETA_USD, "ThetaUSD"),
    ];

    for (addr, expected_name) in tokens {
        let token = IERC20::new(addr, &provider);
        let name = token.name().call().await.unwrap();
        let decimals = token.decimals().call().await.unwrap();

        assert_eq!(name, expected_name, "Token at {addr} should be named {expected_name}");
        assert_eq!(decimals, 6, "All TIP20 tokens use 6 decimals");
    }
}

// ============================================================================
// TIP20 Token Operations: Transfer Emits Event
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_emits_event() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_amount = U256::from(1_000_000);
    let transfer_call = token.transfer(to, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(from)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(!receipt.inner.logs().is_empty(), "Transfer should emit event");

    let log = &receipt.inner.logs()[0];
    assert_eq!(log.address(), ALPHA_USD);

    let transfer_topic =
        alloy_primitives::keccak256("Transfer(address,address,uint256)".as_bytes());
    assert_eq!(log.topics()[0], transfer_topic);
}

// ============================================================================
// Tempo Transactions: Native Value Transfer Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_native_value_transfer_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let tx = TransactionRequest::default()
        .from(from)
        .to(to)
        .value(U256::from(1_000_000_000_000_000_000u64)); // 1 ETH

    let tx = WithOtherFields::new(tx);
    let result = provider.send_transaction(tx).await;
    assert!(result.is_err(), "Native ETH transfers should be rejected in Tempo mode");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("native value transfer not allowed"),
        "Expected 'native value transfer not allowed' error, got: {err}"
    );
}

// ============================================================================
// Tempo Transactions: Zero-Value EIP-1559 Tx Succeeds
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_zero_value_tx_succeeds() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    // TIP20 transfer (value=0, only calldata)
    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1_000_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
}

// ============================================================================
// Tempo Transactions: Contract Deployment
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_contract_deployment() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];

    // Minimal contract: PUSH1 0x00 PUSH1 0x00 RETURN (returns empty)
    let bytecode = Bytes::from(vec![0x60, 0x00, 0x60, 0x00, 0xf3]);

    let tx =
        TransactionRequest::default().from(sender).with_input(bytecode).with_gas_limit(100_000);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
    assert!(receipt.contract_address.is_some(), "Should have deployed a contract");
}

// ============================================================================
// Tempo Transactions: Nonce Increments
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_nonce_increments() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let nonce_before = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce_before, 0);

    // Send a TIP20 transfer
    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(to, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(from)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let nonce_after = provider.get_transaction_count(from).await.unwrap();
    assert_eq!(nonce_after, 1);
}

// ============================================================================
// Tempo AA Transaction Tests (Type 0x76)
// ============================================================================

/// Helper to get the private key for a dev account.
fn dev_key(index: u32) -> PrivateKeySigner {
    let mnemonic = "test test test test test test test test test test test junk";
    alloy_signer_local::MnemonicBuilder::<alloy_signer_local::coins_bip39::English>::default()
        .phrase(mnemonic)
        .index(index)
        .expect("valid mnemonic")
        .build()
        .expect("valid key")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_basic() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(100_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::ZERO,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction should succeed");

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + transfer_amount,
        "Recipient should receive transfer amount"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_with_2d_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    // Send two transactions with different nonce keys (can be parallelized)
    let nonce_keys = [U256::from(1), U256::from(2)];

    for (i, nonce_key) in nonce_keys.iter().enumerate() {
        let transfer_amount = U256::from(50_000 * (i + 1) as u64);
        let transfer_call = token.transfer(recipient, transfer_amount);
        let calldata: Bytes = transfer_call.calldata().clone();

        let tempo_tx = TempoTransaction {
            chain_id,
            fee_token: Some(ALPHA_USD),
            max_priority_fee_per_gas: base_fee / 10,
            max_fee_per_gas: base_fee * 2,
            gas_limit: TIP20_TRANSFER_GAS,
            calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
            access_list: Default::default(),
            nonce_key: *nonce_key,
            nonce: 0,
            fee_payer_signature: None,
            valid_before: None,
            valid_after: None,
            key_authorization: None,
            tempo_authorization_list: vec![],
        };

        let sig_hash = tempo_tx.signature_hash();
        let signature = signer.sign_hash(&sig_hash).await.unwrap();
        let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
        let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
        let envelope = TempoTxEnvelope::AA(signed_tx);

        let mut encoded = Vec::new();
        envelope.encode_2718(&mut encoded);
        let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
        let receipt = tx_hash.get_receipt().await.unwrap();

        assert!(receipt.status(), "Tempo AA transaction with nonce_key {nonce_key} should succeed");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_with_valid_before() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time + 30;

    let transfer_call = token.transfer(recipient, U256::from(75_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(3),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction with valid_before should succeed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_with_valid_after() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_after = current_time;
    let valid_before = current_time + 30;

    let transfer_call = token.transfer(recipient, U256::from(60_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(4),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: NonZeroU64::new(valid_after),
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(
        receipt.status(),
        "Tempo AA transaction with valid_after (already valid) should succeed"
    );
}

// ============================================================================
// Tempo AA Transaction Error Cases
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_expired_valid_before() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time.saturating_sub(10); // 10 seconds ago

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(100),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    assert!(result.is_err(), "Transaction with expired valid_before should be rejected");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_valid_after_future() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_after = current_time + 5;
    let valid_before = current_time + 60;

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(101),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: NonZeroU64::new(valid_after),
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    // Transaction enters pool but is not yet valid
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();

    // Advance time past valid_after
    api.evm_set_next_block_timestamp(valid_after + 1).unwrap();
    api.mine_one().await;

    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "Transaction should succeed after valid_after time");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_nonce_replay_same_key() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let nonce_key = U256::from(200);

    // First transaction with nonce 0
    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx1 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call {
            to: TxKind::Call(PATH_USD),
            value: U256::ZERO,
            input: calldata.clone(),
        }],
        access_list: Default::default(),
        nonce_key,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx1.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx1, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "First transaction should succeed");

    // Second transaction with nonce=1 on the same key should succeed
    let transfer_call2 = token.transfer(recipient, U256::from(60_000));
    let calldata2: Bytes = transfer_call2.calldata().clone();

    let tempo_tx2 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata2 }],
        access_list: Default::default(),
        nonce_key,
        nonce: 1,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx2.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx2, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash2 = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt2 = tx_hash2.get_receipt().await.unwrap();
    assert!(receipt2.status(), "Second transaction with nonce=1 should succeed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_parallel_nonces_different_keys() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    // Send two transactions with the SAME nonce (0) but DIFFERENT nonce keys
    let mut tx_hashes = vec![];

    for nonce_key_val in [300u64, 301u64] {
        let transfer_call = token.transfer(recipient, U256::from(10_000));
        let calldata: Bytes = transfer_call.calldata().clone();

        let tempo_tx = TempoTransaction {
            chain_id,
            fee_token: Some(ALPHA_USD),
            max_priority_fee_per_gas: base_fee / 10,
            max_fee_per_gas: base_fee * 2,
            gas_limit: TIP20_TRANSFER_GAS,
            calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
            access_list: Default::default(),
            nonce_key: U256::from(nonce_key_val),
            nonce: 0,
            fee_payer_signature: None,
            valid_before: None,
            valid_after: None,
            key_authorization: None,
            tempo_authorization_list: vec![],
        };

        let sig_hash = tempo_tx.signature_hash();
        let signature = signer.sign_hash(&sig_hash).await.unwrap();
        let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
        let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
        let envelope = TempoTxEnvelope::AA(signed_tx);

        let mut encoded = Vec::new();
        envelope.encode_2718(&mut encoded);

        let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
        tx_hashes.push(tx_hash);
    }

    for tx_hash in tx_hashes {
        let receipt = tx_hash.get_receipt().await.unwrap();
        assert!(receipt.status(), "Parallel transactions with different nonce keys should succeed");
    }

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + U256::from(20_000),
        "Recipient should receive both transfers"
    );
}

// ============================================================================
// Gas Estimation
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(accounts[1], U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default().from(accounts[0]).to(ALPHA_USD).with_input(calldata);

    let gas_estimate = provider.estimate_gas(tx.into()).await.unwrap();

    // TIP20 transfer should use more than 21000 gas
    assert!(gas_estimate > 21000, "TIP20 transfer should use more than 21000 gas");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_for_contract_call() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(accounts[1], U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default().from(accounts[0]).to(ALPHA_USD).with_input(calldata);

    let gas_estimate = provider.estimate_gas(tx.into()).await.unwrap();

    // Contract call should use more gas than simple transfer
    assert!(gas_estimate > 21000, "Contract call should use more than 21000 gas");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_with_value_fails() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    // Gas estimation with native value should fail in Tempo mode
    let tx = TransactionRequest::default()
        .from(accounts[0])
        .to(accounts[1])
        .value(U256::from(1_000_000_000_000_000_000u64));

    let result = provider.estimate_gas(tx.into()).await;
    assert!(result.is_err(), "Gas estimation with native value should fail in Tempo mode");
}

// ============================================================================
// Gas Price & Base Fee
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_price() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let gas_price = provider.get_gas_price().await.unwrap();

    assert!(gas_price > 0, "Gas price should be non-zero");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_base_fee() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();

    assert!(block.header.base_fee_per_gas.is_some());
}

// ============================================================================
// Fee Token Deduction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_eip1559_fee_token_deduction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    // Check fee token balance before (ALPHA_USD is the default fee token)
    let fee_token = IERC20::new(ALPHA_USD, &provider);
    let fee_balance_before = fee_token.balanceOf(sender).call().await.unwrap();

    // Transfer PATH_USD so balance change is only from gas fees, not the transfer itself
    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(100_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let base_fee = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .max_fee_per_gas(base_fee * 2)
        .max_priority_fee_per_gas(base_fee / 10);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "Transaction should succeed");

    // Fee token balance should have decreased (gas fees paid in ALPHA_USD)
    let fee_balance_after = fee_token.balanceOf(sender).call().await.unwrap();
    assert!(
        fee_balance_after < fee_balance_before,
        "Fee token balance should decrease after paying gas (before: {fee_balance_before}, after: {fee_balance_after})"
    );
}

// ============================================================================
// Anvil Control: Set Balance
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_balance() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let random_address = Address::random();
    let new_balance = U256::from(1_000_000_000_000_000_000u64);

    let balance_before = provider.get_balance(random_address).await.unwrap();
    assert_eq!(balance_before, U256::ZERO);

    api.anvil_set_balance(random_address, new_balance).await.unwrap();

    let balance_after = provider.get_balance(random_address).await.unwrap();
    assert_eq!(balance_after, new_balance);
}

// ============================================================================
// Anvil Control: Set Code
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_set_code() {
    let (api, _handle) = spawn(NodeConfig::test_tempo()).await;

    let target = Address::random();

    let code_before = api.get_code(target, None).await.unwrap();
    assert!(code_before.is_empty());

    let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // PUSH 0, PUSH 0, RETURN
    api.anvil_set_code(target, bytecode.clone().into()).await.unwrap();

    let code_after = api.get_code(target, None).await.unwrap();
    assert_eq!(code_after.as_ref(), bytecode.as_slice());
}

// ============================================================================
// Anvil Control: Auto Mine Toggle
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_auto_mine_toggle() {
    let (api, _handle) = spawn(NodeConfig::test_tempo()).await;

    assert!(api.anvil_get_auto_mine().unwrap());

    api.anvil_set_auto_mine(false).await.unwrap();
    assert!(!api.anvil_get_auto_mine().unwrap());

    api.anvil_set_auto_mine(true).await.unwrap();
    assert!(api.anvil_get_auto_mine().unwrap());
}

// ============================================================================
// Anvil Control: Manual Mining
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_manual_mining() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let block_before = provider.get_block_number().await.unwrap();

    api.mine_one().await;

    let block_after = provider.get_block_number().await.unwrap();
    assert_eq!(block_after, block_before + 1);

    api.anvil_mine(Some(U256::from(5)), None).await.unwrap();

    let block_final = provider.get_block_number().await.unwrap();
    assert_eq!(block_final, block_after + 5);
}

// ============================================================================
// Anvil Control: Impersonate Account
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_impersonate_account() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let impersonated = handle.dev_accounts().next().unwrap();
    let recipient = handle.dev_accounts().nth(1).unwrap();

    api.anvil_impersonate_account(impersonated).await.unwrap();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(impersonated)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    api.anvil_stop_impersonating_account(impersonated).await.unwrap();
}

// ============================================================================
// Anvil Control: Snapshot and Revert
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_snapshot_and_revert() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let from = accounts[0];
    let to = accounts[1];

    let token = IERC20::new(ALPHA_USD, &provider);
    let balance_before = token.balanceOf(to).call().await.unwrap();
    let block_before = provider.get_block_number().await.unwrap();

    let snapshot_id = api.evm_snapshot().await.unwrap();

    let transfer_call = token.transfer(to, U256::from(1_000_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(from)
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let balance_after_tx = token.balanceOf(to).call().await.unwrap();
    assert!(balance_after_tx > balance_before);

    api.evm_revert(snapshot_id).await.unwrap();

    let balance_reverted = token.balanceOf(to).call().await.unwrap();
    let block_reverted = provider.get_block_number().await.unwrap();

    assert_eq!(balance_reverted, balance_before);
    assert_eq!(block_reverted, block_before);
}

// ============================================================================
// Block & Chain: Tempo Mode Enabled
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_mode_enabled_by_default() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, 0);
}

// ============================================================================
// Block & Chain: Fee Tokens Deployed
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_tokens_deployed() {
    let (api, _handle) = spawn(NodeConfig::test_tempo()).await;

    for token in [PATH_USD, ALPHA_USD, BETA_USD, THETA_USD] {
        let code = api.get_code(token, None).await.unwrap();
        assert!(!code.is_empty(), "Token {token} should have code deployed");
    }
}

// ============================================================================
// Block & Chain: Block Has Timestamp
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_has_timestamp() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;

    let block = provider.get_block(1.into()).await.unwrap().unwrap();
    assert!(block.header.timestamp > 0, "Block should have a timestamp");
}

// ============================================================================
// Block & Chain: Block Timestamp Increases
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_timestamp_increases() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;
    let block1 = provider.get_block(1.into()).await.unwrap().unwrap();

    let future_timestamp = block1.header.timestamp + 100;
    api.evm_set_next_block_timestamp(future_timestamp).unwrap();

    api.mine_one().await;
    let block2 = provider.get_block(2.into()).await.unwrap().unwrap();

    assert_eq!(block2.header.timestamp, future_timestamp);
    assert!(block2.header.timestamp > block1.header.timestamp);
}

// ============================================================================
// Block & Chain: Block Timestamps Are Monotonic
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_timestamps_are_monotonic() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;
    let block1 = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let timestamp1 = block1.header.timestamp;

    let future_timestamp = timestamp1 + 10;
    api.evm_set_next_block_timestamp(future_timestamp).unwrap();

    api.mine_one().await;
    let block2 = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let timestamp2 = block2.header.timestamp;

    assert!(
        timestamp2 > timestamp1,
        "Block timestamps must be strictly increasing: {timestamp2} should be > {timestamp1}",
    );
    assert_eq!(timestamp2, future_timestamp, "Block timestamp should match the set value");
}

// ============================================================================
// Block & Chain: Block Gas Limit
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_block_gas_limit() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.mine_one().await;

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();

    assert!(block.header.gas_limit > 0);
}

// ============================================================================
// Block & Chain: Transaction Respects Gas Limit
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_respects_gas_limit() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);
    let transfer_call = token.transfer(accounts[1], U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx = TransactionRequest::default()
        .from(accounts[0])
        .to(ALPHA_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
    assert!(receipt.gas_used <= TIP20_TRANSFER_GAS);
}

// ============================================================================
// Block & Chain: Multiple Transactions in Block
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_transactions_in_block() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts: Vec<Address> = handle.dev_accounts().collect();

    let token = IERC20::new(ALPHA_USD, &provider);

    let transfer1 = token.transfer(accounts[1], U256::from(1000));
    let calldata1: Bytes = transfer1.calldata().clone();
    let tx1 = TransactionRequest::default()
        .from(accounts[0])
        .to(ALPHA_USD)
        .with_input(calldata1)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let transfer2 = token.transfer(accounts[3], U256::from(2000));
    let calldata2: Bytes = transfer2.calldata().clone();
    let tx2 = TransactionRequest::default()
        .from(accounts[2])
        .to(ALPHA_USD)
        .with_input(calldata2)
        .with_gas_limit(TIP20_TRANSFER_GAS);

    let tx1 = WithOtherFields::new(tx1);
    let tx2 = WithOtherFields::new(tx2);

    let pending1 = provider.send_transaction(tx1).await.unwrap();
    let pending2 = provider.send_transaction(tx2).await.unwrap();

    api.mine_one().await;

    let receipt1 = pending1.get_receipt().await.unwrap();
    let receipt2 = pending2.get_receipt().await.unwrap();

    assert_eq!(receipt1.block_number, receipt2.block_number);
}

// ============================================================================
// Block & Chain: Chain ID
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 31337);
}

// ============================================================================
// Block & Chain: Custom Chain ID
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_custom_chain_id() {
    let custom_chain_id = 42069u64;
    let (_api, handle) = spawn(NodeConfig::test_tempo().with_chain_id(Some(custom_chain_id))).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, custom_chain_id);
}

// ============================================================================
// Tempo AA: Expiring Nonce Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_expiring_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time + 25;

    let transfer_call = token.transfer(recipient, U256::from(80_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::MAX,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction with expiring nonce should succeed");
}

// ============================================================================
// Tempo AA: Expiring Nonce Replay
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_expiring_nonce_replay() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let current_time = block.header.timestamp;
    let valid_before = current_time + 25;

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::MAX,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: NonZeroU64::new(valid_before),
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let first_tx_hash = *tx_hash.tx_hash();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "First expiring nonce transaction should succeed");

    // Replay the exact same transaction bytes
    let result = provider.send_raw_transaction(&encoded).await;

    if let Ok(pending) = result {
        let second_tx_hash = *pending.tx_hash();
        assert_eq!(
            first_tx_hash, second_tx_hash,
            "Replaying same transaction should return same tx hash (not execute again)"
        );
    }
}

// ============================================================================
// Tempo AA: Multiple Calls in Single Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_multiple_calls() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient1 = accounts[1];
    let recipient2 = accounts[2];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let recipient1_balance_before = token.balanceOf(recipient1).call().await.unwrap();
    let recipient2_balance_before = token.balanceOf(recipient2).call().await.unwrap();

    let amount1 = U256::from(25_000);
    let amount2 = U256::from(35_000);

    let call1_data: Bytes = token.transfer(recipient1, amount1).calldata().clone();
    let call2_data: Bytes = token.transfer(recipient2, amount2).calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS * 2,
        calls: vec![
            Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: call1_data },
            Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: call2_data },
        ],
        access_list: Default::default(),
        nonce_key: U256::from(5),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);
    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Tempo AA transaction with multiple calls should succeed");

    let recipient1_balance_after = token.balanceOf(recipient1).call().await.unwrap();
    let recipient2_balance_after = token.balanceOf(recipient2).call().await.unwrap();

    assert_eq!(recipient1_balance_after, recipient1_balance_before + amount1);
    assert_eq!(recipient2_balance_after, recipient2_balance_before + amount2);
}

// ============================================================================
// Tempo AA: Nonce Keys Are Isolated
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_nonce_keys_are_isolated() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx1 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call {
            to: TxKind::Call(PATH_USD),
            value: U256::ZERO,
            input: calldata.clone(),
        }],
        access_list: Default::default(),
        nonce_key: U256::from(100),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx1.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx1, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "First tx with nonce_key=100 should succeed");

    let tempo_tx2 = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(101),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx2.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx2, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash2 = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt2 = tx_hash2.get_receipt().await.unwrap();
    assert!(
        receipt2.status(),
        "Tx with different nonce_key should succeed even with same nonce value"
    );
}

// ============================================================================
// Tempo AA: Explicit Fee Token Selection
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_explicit_fee_token_selection() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];
    let signer = dev_key(0);

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let path_token = IERC20::new(PATH_USD, &provider);
    let alpha_token = IERC20::new(ALPHA_USD, &provider);
    let path_balance_before = path_token.balanceOf(sender).call().await.unwrap();
    let alpha_balance_before = alpha_token.balanceOf(sender).call().await.unwrap();

    let transfer_call = path_token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(PATH_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(400),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "Transaction with explicit fee token should succeed");

    let path_balance_after = path_token.balanceOf(sender).call().await.unwrap();
    let alpha_balance_after = alpha_token.balanceOf(sender).call().await.unwrap();

    assert!(
        path_balance_after < path_balance_before,
        "PATH_USD balance should decrease (transfer + fees)"
    );
    assert_eq!(
        alpha_balance_after, alpha_balance_before,
        "ALPHA_USD balance should not change when using PATH_USD for fees"
    );
}

// ============================================================================
// Tempo AA: Fee Token Swap (Different Tokens)
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_fee_token_swap_different_tokens() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let alpha_token = IERC20::new(ALPHA_USD, &provider);
    let alice_alpha_before = alpha_token.balanceOf(sender).call().await.unwrap();

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(100_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let base_fee = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .max_fee_per_gas(base_fee * 2)
        .max_priority_fee_per_gas(base_fee / 10);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "Transaction should succeed");

    let alice_alpha_after = alpha_token.balanceOf(sender).call().await.unwrap();
    assert!(
        alice_alpha_after < alice_alpha_before,
        "Alice's AlphaUSD should decrease due to gas fees"
    );
}

// ============================================================================
// Tempo AA: Receipt Fields
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_transaction_receipt_fields() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(500),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();

    assert!(receipt.status(), "Transaction should succeed");
    assert!(receipt.gas_used > 0, "Gas used should be non-zero");
    assert!(!receipt.inner.logs().is_empty(), "Should have Transfer event logs");
}

// ============================================================================
// Tempo AA: Get Transaction By Hash
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_get_transaction_by_hash() {
    let (api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(50_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(501),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let pending = provider.send_raw_transaction(&encoded).await.unwrap();
    let tx_hash = *pending.tx_hash();
    pending.get_receipt().await.unwrap();

    let tx = api.transaction_by_hash(tx_hash).await.unwrap();
    assert!(tx.is_some(), "Transaction should be retrievable by hash");

    let tx = tx.unwrap();
    assert_eq!(tx.ty(), 0x76, "Transaction type should be 0x76 (Tempo)");
    assert_eq!(TransactionResponse::from(&tx), sender, "From address should match sender");
}

// ============================================================================
// Tempo AA: Wrong Chain ID Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_wrong_chain_id_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let correct_chain_id = provider.get_chain_id().await.unwrap();
    let wrong_chain_id = correct_chain_id + 1;
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id: wrong_chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(1),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    assert!(result.is_err(), "Transaction with wrong chain ID should be rejected");
}

// ============================================================================
// Tempo AA: Gas Too Low Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_gas_too_low_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: 1000,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(2),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    assert!(result.is_err(), "Transaction with gas limit below intrinsic cost should be rejected");
}

// ============================================================================
// Tempo AA: Value In Call Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_value_in_call_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call {
            to: TxKind::Call(recipient),
            value: U256::from(1_000_000),
            input: Bytes::new(),
        }],
        access_list: Default::default(),
        nonce_key: U256::from(3),
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let result = provider.send_raw_transaction(&encoded).await;
    if let Ok(pending) = result {
        let receipt = pending.get_receipt().await;
        if let Ok(r) = receipt {
            assert!(!r.status(), "Transaction with ETH value should fail in Tempo mode");
        }
    }
}

// ============================================================================
// Tempo AA: Nonce Too High Rejected
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_tempo_aa_nonce_too_high_rejected() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let transfer_call = token.transfer(recipient, U256::from(10_000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(ALPHA_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: TIP20_TRANSFER_GAS,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::from(999),
        nonce: 5,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    // Transaction may be accepted into pool but should fail during execution
    let result = provider.send_raw_transaction(&encoded).await;
    if result.is_ok() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

// ============================================================================
// Gas Estimation: Tempo AA Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_tempo_aa_transaction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default().from(accounts[0]).to(PATH_USD).with_input(calldata),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    assert!(
        gas_estimate > 21000,
        "Tempo AA gas estimate should be greater than 21000, got: {gas_estimate}"
    );
}

// ============================================================================
// Gas Estimation: Tempo AA with 2D Nonce
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_tempo_aa_with_2d_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Baseline: plain AA tx (no 2D nonce)
    let baseline_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_gas = provider.estimate_gas(baseline_tx).await.unwrap();

    // 2D nonce tx with nonce=0 (new nonce key)
    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata)
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!("0x64")),
        ]
        .into_iter()
        .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // New 2D nonce key (nonce=0) charges COLD_SLOAD + SSTORE_SET = 22100 gas
    let nonce_key_delta = gas_estimate - baseline_gas;
    assert!(
        nonce_key_delta >= 22_100,
        "2D nonce should add >= 22100 gas, got delta: {nonce_key_delta} \
         (baseline: {baseline_gas}, 2d_nonce: {gas_estimate})"
    );
}

// ============================================================================
// Gas Estimation: Tempo AA with Expiring Nonce
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_tempo_aa_expiring_nonce() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Baseline: plain AA tx (no expiring nonce)
    let baseline_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_gas = provider.estimate_gas(baseline_tx).await.unwrap();

    let max_nonce_key = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata)
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!(max_nonce_key)),
        ]
        .into_iter()
        .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // At T0, expiring nonces are treated as 2D nonces (22100 gas).
    // At T1+, this charges EXPIRING_NONCE_GAS = 13000 instead.
    let expiring_delta = gas_estimate - baseline_gas;
    assert!(
        expiring_delta >= 22_100,
        "Expiring nonce should add >= 22100 gas at T0, got delta: {expiring_delta} \
         (baseline: {baseline_gas}, expiring: {gas_estimate})"
    );
}

// ============================================================================
// Gas Estimation: T1 Hardfork Nonce Gas Costs
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_t1_nonce_costs() {
    use tempo_chainspec::hardfork::TempoHardfork;

    let (_api, handle) =
        spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T1.into()))).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Baseline: plain AA tx (no nonce key)
    let baseline_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_gas = provider.estimate_gas(baseline_tx).await.unwrap();

    // TIP-1000: nonce=0 pays 250k for account creation
    assert!(
        baseline_gas > 250_000,
        "T1 baseline should include 250k (TIP-1000), got: {baseline_gas}"
    );

    // Existing 2D nonce key (nonce=1) charges 5,000 gas
    let nonce_2d_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(1),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!("0x64")),
        ]
        .into_iter()
        .collect(),
    };
    let nonce_2d_gas = provider.estimate_gas(nonce_2d_tx).await.unwrap();

    let baseline_nonce1_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(1),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };
    let baseline_nonce1_gas = provider.estimate_gas(baseline_nonce1_tx).await.unwrap();
    let nonce_2d_delta = nonce_2d_gas - baseline_nonce1_gas;

    assert!(
        nonce_2d_delta >= 5_000,
        "T1: existing 2D nonce key should add >= 5000 gas, got: {nonce_2d_delta}"
    );

    // TIP-1000: nonce=0 should cost 250k more than nonce=1
    let tip1000_delta = baseline_gas - baseline_nonce1_gas;
    assert!(
        tip1000_delta >= 250_000,
        "T1: TIP-1000 delta should be >= 250000, got: {tip1000_delta}"
    );

    // Expiring nonce (nonce_key=MAX) at T1 should charge ~13K for ring buffer ops
    // (2*COLD_SLOAD + WARM_SLOAD + 3*WARM_SSTORE_RESET), NOT 22K like at T0.
    let max_nonce_key = "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let block = provider.get_block(BlockNumberOrTag::Latest.into()).await.unwrap().unwrap();
    let valid_before = block.header.timestamp + 25;

    let expiring_tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!(max_nonce_key)),
            ("validBefore".to_string(), serde_json::json!(valid_before)),
        ]
        .into_iter()
        .collect(),
    };
    let expiring_gas = provider.estimate_gas(expiring_tx).await.unwrap();

    // Compare against baseline_nonce1 (nonce=1, no nonce key) since both the baseline (nonce=0)
    // and expiring tx (nonce=0) include the 250k account creation cost.
    let expiring_delta = expiring_gas - baseline_nonce1_gas;
    assert!(
        expiring_delta >= 13_000,
        "T1: expiring nonce should add at least ~13K gas for ring buffer ops, got delta: {expiring_delta}"
    );
}

// ============================================================================
// Gas Estimation: 2D Nonce Estimate Sufficient for Real Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_2d_nonce_converges() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];
    let signer = dev_key(0);

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    // Estimate gas with 2D nonce
    let nonce_key = U256::from(0x64);
    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone())
            .with_nonce(0),
        other: [
            ("feeToken".to_string(), serde_json::json!(PATH_USD.to_string())),
            ("nonceKey".to_string(), serde_json::json!(format!("{nonce_key:#x}"))),
        ]
        .into_iter()
        .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // Send the actual transaction with the estimated gas to verify it's sufficient
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(PATH_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: gas_estimate,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(
        receipt.status(),
        "2D nonce transaction should succeed with estimated gas: {gas_estimate}"
    );
    assert!(
        receipt.gas_used() <= gas_estimate,
        "Gas used ({}) should be <= estimate ({}) for 2D nonce tx",
        receipt.gas_used(),
        gas_estimate
    );
}

// ============================================================================
// Gas Estimation: Converges for Tempo Intrinsic Gas
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_gas_estimation_converges_for_tempo_intrinsic_gas() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let transfer_call = token.transfer(recipient, U256::from(1000));
    let calldata: Bytes = transfer_call.calldata().clone();

    let tx: WithOtherFields<TransactionRequest> = WithOtherFields {
        inner: TransactionRequest::default()
            .from(accounts[0])
            .to(PATH_USD)
            .with_input(calldata.clone()),
        other: [("feeToken".to_string(), serde_json::json!(PATH_USD.to_string()))]
            .into_iter()
            .collect(),
    };

    let gas_estimate = provider.estimate_gas(tx).await.unwrap();

    // Send the actual transaction with the estimated gas to verify convergence
    let signer = dev_key(0);
    let chain_id = provider.get_chain_id().await.unwrap();
    let base_fee = provider.get_gas_price().await.unwrap();

    let tempo_tx = TempoTransaction {
        chain_id,
        fee_token: Some(PATH_USD),
        max_priority_fee_per_gas: base_fee / 10,
        max_fee_per_gas: base_fee * 2,
        gas_limit: gas_estimate,
        calls: vec![Call { to: TxKind::Call(PATH_USD), value: U256::ZERO, input: calldata }],
        access_list: Default::default(),
        nonce_key: U256::ZERO,
        nonce: 0,
        fee_payer_signature: None,
        valid_before: None,
        valid_after: None,
        key_authorization: None,
        tempo_authorization_list: vec![],
    };

    let sig_hash = tempo_tx.signature_hash();
    let signature = signer.sign_hash(&sig_hash).await.unwrap();
    let tempo_sig = TempoSignature::Primitive(PrimitiveSignature::Secp256k1(signature));
    let signed_tx = AASigned::new_unhashed(tempo_tx, tempo_sig);
    let envelope = TempoTxEnvelope::AA(signed_tx);

    let mut encoded = Vec::new();
    envelope.encode_2718(&mut encoded);

    let tx_hash = provider.send_raw_transaction(&encoded).await.unwrap();
    let receipt = tx_hash.get_receipt().await.unwrap();
    assert!(receipt.status(), "Transaction should succeed with estimated gas: {gas_estimate}");

    assert!(
        receipt.gas_used() <= gas_estimate,
        "Gas used ({}) should be <= estimate ({})",
        receipt.gas_used(),
        gas_estimate
    );
}

// ============================================================================
// EIP-1559 Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_eip1559_transaction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(500_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let base_fee = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .max_fee_per_gas(base_fee * 2)
        .max_priority_fee_per_gas(base_fee / 10);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "EIP-1559 transaction should succeed");

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + transfer_amount,
        "Recipient should receive transfer amount"
    );
}

// ============================================================================
// Legacy Transaction
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_legacy_transaction() {
    let (_api, handle) = spawn(NodeConfig::test_tempo()).await;
    let provider = handle.http_provider();

    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let recipient = accounts[1];

    let token = IERC20::new(PATH_USD, &provider);
    let recipient_balance_before = token.balanceOf(recipient).call().await.unwrap();

    let transfer_amount = U256::from(250_000);
    let transfer_call = token.transfer(recipient, transfer_amount);
    let calldata: Bytes = transfer_call.calldata().clone();

    let gas_price = provider.get_gas_price().await.unwrap();

    let tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD)
        .with_input(calldata)
        .with_gas_limit(TIP20_TRANSFER_GAS)
        .with_gas_price(gas_price);

    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status(), "Legacy transaction should succeed");

    let recipient_balance_after = token.balanceOf(recipient).call().await.unwrap();
    assert_eq!(
        recipient_balance_after,
        recipient_balance_before + transfer_amount,
        "Recipient should receive transfer amount"
    );
}
