use std::collections::HashMap;

use crate::{
    abi::{Multicall, SimpleStorage},
    fork::fork_config,
    utils::http_provider_with_signer,
};
use alloy_eips::BlockId;
use alloy_network::{AnyNetwork, EthereumWallet, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{
    Address, B256, Bytes, U256, address,
    hex::{self, FromHex},
};
use alloy_provider::{
    Provider,
    ext::{DebugApi, TraceApi},
};
use alloy_rlp::Encodable;
use alloy_rpc_types::{
    BlockNumberOrTag, Index, TransactionRequest,
    state::StateOverride,
    trace::{
        filter::{TraceFilter, TraceFilterMode},
        geth::{
            AccountState, CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType,
            GethDebugTracingCallOptions, GethDebugTracingOptions, GethDefaultTracingOptions,
            GethTrace, PreStateConfig, PreStateFrame, TraceResult,
        },
        opcode::{BlockOpcodeGas, TransactionOpcodeGas},
        parity::{
            Action, ChangedType, LocalizedTransactionTrace, TraceResults,
            TraceResultsWithTransactionHash, TraceType,
        },
    },
};
use alloy_rpc_types_eth::AccountInfo;
use alloy_serde::WithOtherFields;
use alloy_sol_types::{SolCall, SolValue, sol};
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;
use revm::context_interface::block::BlobExcessGasAndPrice;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transfer_parity_traces() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());

    match traces[0].trace.action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
            assert_eq!(call.value, amount);
        }
        _ => unreachable!("unexpected action"),
    }

    let num = provider.get_block_number().await.unwrap();
    let block_traces = provider.trace_block(num.into()).await.unwrap();
    assert!(!block_traces.is_empty());

    assert_eq!(traces, block_traces);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_transaction_opcode_gas_local() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let signer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let storage = SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();
    let receipt =
        storage.setValue("bar".to_string()).send().await.unwrap().get_receipt().await.unwrap();
    let opcode_gas: Option<TransactionOpcodeGas> = handle
        .http_provider()
        .raw_request("trace_transactionOpcodeGas".into(), (receipt.transaction_hash,))
        .await
        .unwrap();
    let opcode_gas = opcode_gas.unwrap();

    assert_eq!(opcode_gas.transaction_hash, receipt.transaction_hash);
    assert!(opcode_gas.contains("SSTORE"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_block_opcode_gas_local() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let signer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let genesis = provider.get_block(BlockId::number(0)).await.unwrap().unwrap();
    for block_id in [BlockId::number(0), BlockId::hash(genesis.header.hash), BlockId::earliest()] {
        let opcode_gas: Option<BlockOpcodeGas> = handle
            .http_provider()
            .raw_request("trace_blockOpcodeGas".into(), (block_id,))
            .await
            .unwrap();
        let opcode_gas = opcode_gas.unwrap();

        assert_eq!(opcode_gas.block_hash, genesis.header.hash);
        assert_eq!(opcode_gas.block_number, genesis.header.number);
        assert!(opcode_gas.transactions.is_empty());
    }

    let storage = SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();
    let receipt =
        storage.setValue("bar".to_string()).send().await.unwrap().get_receipt().await.unwrap();
    let block_number = receipt.block_number.unwrap();
    let block_hash = receipt.block_hash.unwrap();

    let by_number: Option<BlockOpcodeGas> = handle
        .http_provider()
        .raw_request("trace_blockOpcodeGas".into(), (BlockId::number(block_number),))
        .await
        .unwrap();
    let by_hash: Option<BlockOpcodeGas> = handle
        .http_provider()
        .raw_request("trace_blockOpcodeGas".into(), (BlockId::hash(block_hash),))
        .await
        .unwrap();

    let by_number = by_number.unwrap();
    let by_hash = by_hash.unwrap();

    assert_eq!(by_number.block_hash, block_hash);
    assert_eq!(by_number.block_number, block_number);
    assert_eq!(by_number.transactions.len(), 1);
    assert_eq!(by_number.transactions[0].transaction_hash, receipt.transaction_hash);
    assert!(by_number.contains("SSTORE"));

    assert_eq!(by_hash.block_hash, block_hash);
    assert_eq!(by_hash.block_number, block_number);
    assert_eq!(by_hash.transactions.len(), 1);
    assert_eq!(by_hash.transactions[0].transaction_hash, receipt.transaction_hash);
    assert!(by_hash.contains("SSTORE"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_block_opcode_gas_latest_at_fork_point() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_accounts = origin_handle.dev_wallets().collect::<Vec<_>>();
    let origin_signer: EthereumWallet = origin_accounts[0].clone().into();
    let origin_provider = http_provider_with_signer(&origin_handle.http_endpoint(), origin_signer);
    let storage = SimpleStorage::deploy(&origin_provider, "init value".to_string()).await.unwrap();
    let receipt = storage
        .setValue("fork point".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let fork_point_number = receipt.block_number.unwrap();
    let fork_point_hash = receipt.block_hash.unwrap();

    let (_api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let provider = handle.http_provider();

    // Advance the upstream head after the fork captures its snapshot.
    let advanced =
        storage.setValue("advanced".to_string()).send().await.unwrap().get_receipt().await.unwrap();
    assert!(advanced.block_number.unwrap() > fork_point_number);
    assert_eq!(provider.get_block_number().await.unwrap(), fork_point_number);

    let mut by_latest = provider
        .raw_request::<_, Option<BlockOpcodeGas>>(
            "trace_blockOpcodeGas".into(),
            (BlockId::latest(),),
        )
        .await
        .unwrap()
        .unwrap();
    let mut by_number = provider
        .raw_request::<_, Option<BlockOpcodeGas>>(
            "trace_blockOpcodeGas".into(),
            (BlockId::number(fork_point_number),),
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(by_number.block_hash, fork_point_hash);
    assert_eq!(by_number.block_number, fork_point_number);
    assert_eq!(by_number.transactions.len(), 1);
    assert_eq!(by_number.transactions[0].transaction_hash, receipt.transaction_hash);
    assert_eq!(by_latest.block_number, by_number.block_number);
    assert_eq!(by_latest.block_hash, by_number.block_hash);
    // Opcode gas entries are collected from a map and have no stable order.
    for transaction in by_latest.transactions.iter_mut().chain(&mut by_number.transactions) {
        transaction.opcode_gas.sort_unstable_by(|a, b| a.opcode.cmp(&b.opcode));
    }
    assert_eq!(by_latest.transactions, by_number.transactions);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_raw_transaction_local() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let value = U256::from(1_000_000_000u64);
    let from_balance = provider.get_balance(from).await.unwrap();
    let to_balance = provider.get_balance(to).await.unwrap();

    let tx = TransactionRequest::default()
        .from(from)
        .to(to)
        .value(value)
        .with_gas_limit(21_000)
        .max_fee_per_gas(20_000_000_000)
        .max_priority_fee_per_gas(1_000_000_000);
    let signed_tx = api.sign_transaction(WithOtherFields::new(tx)).await.unwrap();
    let raw_tx = hex::decode(&signed_tx[2..]).unwrap();

    let traces =
        provider.trace_raw_transaction(&raw_tx).trace().state_diff().vm_trace().await.unwrap();

    assert!(traces.state_diff.is_some());
    assert!(traces.vm_trace.is_some());
    assert!(!traces.trace.is_empty());
    match traces.trace[0].action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
            assert_eq!(call.value, value);
        }
        _ => unreachable!("unexpected action"),
    }

    assert_eq!(provider.get_transaction_count(from).await.unwrap(), 0);
    assert_eq!(provider.get_balance(from).await.unwrap(), from_balance);
    assert_eq!(provider.get_balance(to).await.unwrap(), to_balance);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_raw_transaction_rejects_code_sender() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let tx = TransactionRequest::default()
        .from(from)
        .to(accounts[1].address())
        .value(U256::ONE)
        .with_gas_limit(21_000)
        .max_fee_per_gas(20_000_000_000)
        .max_priority_fee_per_gas(1_000_000_000);
    let signed_tx = api.sign_transaction(WithOtherFields::new(tx)).await.unwrap();
    let raw_tx = hex::decode(&signed_tx[2..]).unwrap();

    api.anvil_set_code(from, Bytes::from_static(&[0x00])).await.unwrap();
    let error = provider.trace_raw_transaction(&raw_tx).trace().await.unwrap_err();
    let error = error.as_error_resp().unwrap();
    assert_eq!((error.code, error.message.as_ref()), (-32003, "sender not an eoa"));

    // An EIP-7702 delegation keeps the account an EOA.
    let delegation = [&[0xef, 0x01, 0x00][..], accounts[2].address().as_slice()].concat();
    api.anvil_set_code(from, delegation.into()).await.unwrap();
    let traces = provider.trace_raw_transaction(&raw_tx).trace().await.unwrap();
    assert_eq!(traces.trace.len(), 1);

    // Mining keeps EIP-3607 disabled, so the code sender's transaction is still included.
    api.anvil_set_code(from, Bytes::from_static(&[0x00])).await.unwrap();
    let receipt =
        provider.send_raw_transaction(&raw_tx).await.unwrap().get_receipt().await.unwrap();
    assert!(receipt.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_account_info_at_local() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = U256::from(1000);

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let block_number = receipt.block_number.unwrap();
    let block_hash = receipt.block_hash.unwrap();

    let by_number: Option<AccountInfo> = provider
        .raw_request(
            "debug_accountInfoAt".into(),
            (BlockId::number(block_number), Index::from(0), to),
        )
        .await
        .unwrap();
    let by_hash: Option<AccountInfo> = provider
        .raw_request("debug_accountInfoAt".into(), (BlockId::hash(block_hash), Index::from(0), to))
        .await
        .unwrap();

    let expected_balance = handle.genesis_balance().saturating_add(amount);
    for account in [by_number.unwrap(), by_hash.unwrap()] {
        assert_eq!(account.balance, expected_balance);
        assert_eq!(account.nonce, 0);
        assert!(account.code.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_account_info_at_local_block_on_fork() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_provider = origin_handle.http_provider();
    let origin_accounts = origin_handle.dev_wallets().collect::<Vec<_>>();
    let origin_signer: EthereumWallet = origin_accounts[0].clone().into();
    let origin_provider_with_signer =
        http_provider_with_signer(&origin_handle.http_endpoint(), origin_signer);
    let storage = SimpleStorage::deploy(&origin_provider_with_signer, "init value".to_string())
        .await
        .unwrap();
    let fork_account = *storage.address();
    let expected_code = origin_provider.get_code_at(fork_account).await.unwrap();

    let (_api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let tx = TransactionRequest::default().to(to).value(U256::from(1000)).from(from);
    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let block_number = receipt.block_number.unwrap();

    let account: Option<AccountInfo> = provider
        .raw_request(
            "debug_accountInfoAt".into(),
            (BlockId::number(block_number), Index::from(0), fork_account),
        )
        .await
        .unwrap();
    let account = account.unwrap();

    assert_eq!(account.balance, U256::ZERO);
    assert_eq!(account.nonce, 1);
    assert_eq!(account.code, expected_code);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_account_info_at_delegates_pre_fork_block() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin_provider = origin_handle.http_provider();
    let origin_accounts = origin_handle.dev_wallets().collect::<Vec<_>>();
    let from = origin_accounts[0].address();
    let to = origin_accounts[1].address();
    let amount = U256::from(1000);
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    origin_provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let (_api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let provider = handle.http_provider();

    let account: Option<AccountInfo> = provider
        .raw_request("debug_accountInfoAt".into(), (BlockId::number(1), Index::from(0), to))
        .await
        .unwrap();
    let account = account.unwrap();

    assert_eq!(account.balance, origin_handle.genesis_balance().saturating_add(amount));
    assert_eq!(account.nonce, 0);
    assert!(account.code.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_local() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = U256::from(1000);
    let before = provider.get_balance(to).await.unwrap();
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);

    let traces: TraceResults = provider
        .client()
        .request(
            "trace_call",
            (
                tx,
                vec![TraceType::Trace, TraceType::VmTrace, TraceType::StateDiff],
                BlockId::latest(),
            ),
        )
        .await
        .unwrap();

    assert!(!traces.trace.is_empty());
    assert!(traces.vm_trace.is_some());
    assert!(traces.state_diff.is_some());

    match &traces.trace[0].action {
        Action::Call(call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
            assert_eq!(call.value, amount);
        }
        action => panic!("expected call action, got {action:?}"),
    }

    let ChangedType { from: before_diff, to: after_diff } =
        traces.state_diff.as_ref().unwrap().get(&to).unwrap().balance.as_changed().unwrap();
    assert_eq!(*before_diff, before);
    assert_eq!(after_diff.saturating_sub(*before_diff), amount);

    let after = provider.get_balance(to).await.unwrap();
    assert_eq!(after, before);
}

/// Traces `tx` against the latest block and returns its Parity `trace` and `stateDiff` results.
async fn trace_call_state_diff(
    provider: &impl Provider<AnyNetwork>,
    tx: TransactionRequest,
) -> TraceResults {
    provider
        .client()
        .request(
            "trace_call",
            (
                WithOtherFields::new(tx),
                vec![TraceType::Trace, TraceType::StateDiff],
                BlockId::latest(),
            ),
        )
        .await
        .unwrap()
}

/// Returns the `stateDiff` entry for `address` as JSON, or `None` if it isn't in the diff.
fn state_diff_entry(traces: &TraceResults, address: Address) -> Option<serde_json::Value> {
    let diff = traces.state_diff.as_ref().unwrap().get(&address)?;
    Some(serde_json::to_value(diff).unwrap())
}

/// Returns the `stateDiff` entry of an account born with the given fields.
fn added_account(balance: &str, nonce: &str, code: &str) -> serde_json::Value {
    json!({"balance": {"+": balance}, "nonce": {"+": nonce}, "code": {"+": code}, "storage": {}})
}

/// Returns the `stateDiff` entry of an otherwise unchanged account whose balance changed.
fn changed_balance(from: &str, to: &str) -> serde_json::Value {
    json!({
        "balance": {"*": {"from": from, "to": to}},
        "nonce": "=",
        "code": "=",
        "storage": {},
    })
}

/// PUSH1 0x2a PUSH1 0 MSTORE8 PUSH1 1 PUSH1 0 RETURN: deploys the runtime code `0x2a`.
const INIT_RETURNING_2A: &str = "0x602a60005360016000f3";
/// Stores 0x2a at slot zero, then deploys the runtime code `0x2a`.
const INIT_STORING_AND_RETURNING_2A: &str = "0x602a600055602a60005360016000f3";

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_state_diff_created_account() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();
    let nonce = provider.get_transaction_count(from).await.unwrap();
    let created = from.create(nonce);

    // PUSH1 0 PUSH1 0 RETURN deploys empty code, which is still marked as added.
    for (init, code) in [("0x60006000f3", "0x"), (INIT_RETURNING_2A, "0x2a")] {
        let tx = TransactionRequest::default()
            .from(from)
            .with_deploy_code(Bytes::from_hex(init).unwrap());
        let traces = trace_call_state_diff(&provider, tx).await;
        assert_eq!(state_diff_entry(&traces, created), Some(added_account("0x0", "0x1", code)));
    }

    // A successful CREATE at a prefunded address changes the existing account, including storage.
    let prefunded = from.create(nonce + 1);
    let fund = TransactionRequest::default().from(from).to(prefunded).value(U256::from(7));
    provider
        .send_transaction(WithOtherFields::new(fund))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let tx = TransactionRequest::default()
        .from(from)
        .with_deploy_code(Bytes::from_hex(INIT_STORING_AND_RETURNING_2A).unwrap());
    let traces = trace_call_state_diff(&provider, tx).await;
    assert_eq!(
        state_diff_entry(&traces, prefunded),
        Some(json!({
            "balance": "=",
            "nonce": {"*": {"from": "0x0", "to": "0x1"}},
            "code": {"*": {"from": "0x", "to": "0x2a"}},
            "storage": {
                (B256::ZERO.to_string()): {"*": {
                    "from": B256::ZERO.to_string(),
                    "to": B256::with_last_byte(42).to_string(),
                }},
            },
        }))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_state_diff_funded_account() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();

    let fresh = Address::repeat_byte(0x44);
    let tx = TransactionRequest::default().from(from).to(fresh).value(U256::from(7));
    let traces = trace_call_state_diff(&provider, tx).await;
    assert_eq!(state_diff_entry(&traces, fresh), Some(added_account("0x7", "0x0", "0x")));

    // A zero-value call leaves the account absent.
    let tx = TransactionRequest::default().from(from).to(fresh);
    let traces = trace_call_state_diff(&provider, tx).await;
    assert_eq!(state_diff_entry(&traces, fresh), None);

    // An account that already exists is changed, not added.
    let funded = accounts[1].address();
    let before = provider.get_balance(funded).await.unwrap();
    let tx = TransactionRequest::default().from(from).to(funded).value(U256::from(7));
    let traces = trace_call_state_diff(&provider, tx).await;
    assert_eq!(
        state_diff_entry(&traces, funded),
        Some(changed_balance(&format!("{before:#x}"), &format!("{:#x}", before + U256::from(7)),))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_state_diff_funded_account_fork() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let (_api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let from = handle.dev_wallets().next().unwrap().address();

    // The fork backend also returns an empty account for an address the upstream doesn't have.
    let fresh = Address::repeat_byte(0x44);
    let tx = TransactionRequest::default().from(from).to(fresh).value(U256::from(7));
    let traces = trace_call_state_diff(&handle.http_provider(), tx).await;
    assert_eq!(state_diff_entry(&traces, fresh), Some(added_account("0x7", "0x0", "0x")));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_state_diff_create_and_selfdestruct() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();
    let nonce = provider.get_transaction_count(from).await.unwrap();

    // CALLER SELFDESTRUCT: the account is absent before and after the transaction.
    let tx = TransactionRequest::default()
        .from(from)
        .with_deploy_code(Bytes::from_hex("0x33ff").unwrap());
    let traces = trace_call_state_diff(&provider, tx.clone()).await;
    assert!(traces.trace.iter().any(|trace| trace.action.is_selfdestruct()));
    assert!(traces.state_diff.as_ref().unwrap().contains_key(&from));
    assert_eq!(state_diff_entry(&traces, from.create(nonce)), None);

    // If the address was funded beforehand, the account is deleted.
    let prefunded = from.create(nonce + 1);
    let fund = TransactionRequest::default().from(from).to(prefunded).value(U256::from(7));
    provider
        .send_transaction(WithOtherFields::new(fund))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let traces = trace_call_state_diff(&provider, tx).await;
    assert!(traces.trace.iter().any(|trace| trace.action.is_selfdestruct()));
    assert_eq!(
        state_diff_entry(&traces, prefunded),
        Some(json!({
            "balance": {"-": "0x7"},
            "nonce": {"-": "0x0"},
            "code": {"-": "0x"},
            "storage": {},
        }))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_raw_transaction_state_diff_funded_account() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let from = handle.dev_wallets().next().unwrap().address();

    let fresh = Address::repeat_byte(0x44);
    let tx = TransactionRequest::default()
        .from(from)
        .to(fresh)
        .value(U256::from(7))
        .with_gas_limit(21_000)
        .max_fee_per_gas(20_000_000_000)
        .max_priority_fee_per_gas(1_000_000_000);
    let signed_tx = api.sign_transaction(WithOtherFields::new(tx)).await.unwrap();
    let raw_tx = hex::decode(&signed_tx[2..]).unwrap();

    let traces = handle.http_provider().trace_raw_transaction(&raw_tx).state_diff().await.unwrap();
    assert_eq!(state_diff_entry(&traces, fresh), Some(added_account("0x7", "0x0", "0x")));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_many_state_diff_funded_account() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let from = handle.dev_wallets().next().unwrap().address();
    let fresh = Address::repeat_byte(0x44);

    let tx = TransactionRequest::default().from(from).to(fresh).value(U256::from(7));
    let calls = [
        (WithOtherFields::new(tx.clone()), [TraceType::StateDiff].as_slice()),
        (WithOtherFields::new(tx), [TraceType::StateDiff].as_slice()),
    ];
    let traces = handle.http_provider().trace_call_many(&calls).await.unwrap();

    assert_eq!(state_diff_entry(&traces[0], fresh), Some(added_account("0x7", "0x0", "0x")));
    // The second call sees the account the first one created.
    assert_eq!(state_diff_entry(&traces[1], fresh), Some(changed_balance("0x7", "0xe")));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_state_diff_selfdestruct_hardfork() {
    let contract = Address::repeat_byte(0x55);

    for (hardfork, expected) in [
        (
            EthereumHardfork::Shanghai,
            json!({
                "balance": {"-": "0x7"},
                "nonce": {"-": "0x0"},
                "code": {"-": "0x33ff"},
                "storage": {},
            }),
        ),
        (EthereumHardfork::Cancun, changed_balance("0x7", "0x0")),
    ] {
        let (api, handle) = spawn(NodeConfig::test().with_hardfork(Some(hardfork.into()))).await;
        let provider = handle.http_provider();
        let from = handle.dev_wallets().next().unwrap().address();
        api.anvil_set_code(contract, Bytes::from_hex("0x33ff").unwrap()).await.unwrap();
        api.anvil_set_balance(contract, U256::from(7)).await.unwrap();
        api.anvil_set_storage_at(contract, U256::ZERO, B256::with_last_byte(42)).await.unwrap();

        let traces =
            trace_call_state_diff(&provider, TransactionRequest::default().from(from).to(contract))
                .await;
        assert!(traces.trace.iter().any(|trace| trace.action.is_selfdestruct()));
        assert_eq!(state_diff_entry(&traces, contract), Some(expected), "{hardfork}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_safe_at_fork_point() {
    let epoch = 1u64;
    let (_origin_api, origin_handle) =
        spawn(NodeConfig::test().with_slots_in_an_epoch(epoch)).await;
    let origin_provider = origin_handle.http_provider();
    let origin_accounts = origin_handle.dev_wallets().collect::<Vec<_>>();
    let from = origin_accounts[0].address();
    let to = origin_accounts[1].address();
    let amount1 = U256::from(1_000);
    let amount2 = U256::from(2_000);
    let amount3 = U256::from(3_000);

    // Block 1: `to` balance = genesis + amount1.
    let tx1 = TransactionRequest::default().to(to).value(amount1).from(from);
    origin_provider
        .send_transaction(WithOtherFields::new(tx1))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // Block 2: the fork point. `to` balance = genesis + amount1 + amount2.
    let tx2 = TransactionRequest::default().to(to).value(amount2).from(from);
    let receipt2 = origin_provider
        .send_transaction(WithOtherFields::new(tx2))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let fork_point_number = receipt2.block_number.unwrap();
    assert_eq!(fork_point_number, 2);

    let (_api, handle) = spawn(
        NodeConfig::test()
            .with_slots_in_an_epoch(epoch)
            .with_eth_rpc_url(Some(origin_handle.http_endpoint())),
    )
    .await;
    let provider = handle.http_provider();
    assert_eq!(provider.get_block_number().await.unwrap(), fork_point_number);

    // Advance the upstream head after the fork captures its snapshot: `to` balance becomes
    // genesis + amount1 + amount2 + amount3 on the ORIGIN chain (block 3), while the forked
    // node's own local chain has not advanced past the fork point.
    let tx3 = TransactionRequest::default().to(to).value(amount3).from(from);
    let receipt3 = origin_provider
        .send_transaction(WithOtherFields::new(tx3))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert_eq!(receipt3.block_number.unwrap(), fork_point_number + 1);
    assert_eq!(provider.get_block_number().await.unwrap(), fork_point_number);

    let call_amount = U256::ONE;
    let call =
        WithOtherFields::new(TransactionRequest::default().to(to).value(call_amount).from(from));

    // `safe` resolves locally (current=2, epoch=1) to block 1 - the fork's own snapshot,
    // never the upstream chain's post-fork block 2.
    let by_safe: TraceResults = provider
        .client()
        .request("trace_call", (call.clone(), vec![TraceType::StateDiff], BlockId::safe()))
        .await
        .unwrap();
    let by_number: TraceResults = provider
        .client()
        .request("trace_call", (call, vec![TraceType::StateDiff], BlockId::number(1)))
        .await
        .unwrap();

    let ChangedType { from: before_safe, to: after_safe } =
        by_safe.state_diff.as_ref().unwrap().get(&to).unwrap().balance.as_changed().unwrap();
    let ChangedType { from: before_number, to: after_number } =
        by_number.state_diff.as_ref().unwrap().get(&to).unwrap().balance.as_changed().unwrap();

    let expected_balance_at_block_1 = origin_handle.genesis_balance().saturating_add(amount1);
    assert_eq!(*before_number, expected_balance_at_block_1);
    assert_eq!(
        *before_safe, expected_balance_at_block_1,
        "trace_call(safe) drifted to the upstream tip's resolution instead of the fork's own \
         local resolution"
    );
    assert_eq!(before_safe, before_number);
    assert_eq!(after_safe, after_number);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_many_local() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let storage = SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();
    let set_value = storage.setValue("bar".to_string());
    let get_value = storage.getValue();

    let set_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*storage.address())
        .with_input(set_value.calldata().to_owned());
    let get_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*storage.address())
        .with_input(get_value.calldata().to_owned());
    let trace_types = [TraceType::Trace];
    let calls = [
        (WithOtherFields::new(set_tx), trace_types.as_slice()),
        (WithOtherFields::new(get_tx), trace_types.as_slice()),
    ];

    let traces = handle.http_provider().trace_call_many(&calls).await.unwrap();

    assert_eq!(traces.len(), 2);
    assert!(!traces[0].trace.is_empty());
    assert!(!traces[1].trace.is_empty());

    let traced_value = SimpleStorage::getValueCall::abi_decode_returns(&traces[1].output).unwrap();
    assert_eq!(traced_value, "bar".to_string());
    assert_eq!(storage.getValue().call().await.unwrap(), "init value".to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_empty_trace_types() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    // NUMBER PUSH0 MSTORE PUSH1 0x20 PUSH0 RETURN
    let tx = TransactionRequest::default()
        .from(from)
        .with_deploy_code(Bytes::from_hex("0x435f5260205ff3").unwrap());
    let traces: TraceResults = provider
        .client()
        .request(
            "trace_call",
            (WithOtherFields::new(tx), Vec::<TraceType>::new(), BlockId::latest()),
        )
        .await
        .unwrap();

    assert!(traces.trace.is_empty());
    assert!(traces.vm_trace.is_none());
    assert!(traces.state_diff.is_none());
    assert_eq!(traces.output.len(), 32);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_call_many_defaults_to_latest() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    // NUMBER PUSH0 MSTORE PUSH1 0x20 PUSH0 RETURN
    let tx = TransactionRequest::default()
        .from(from)
        .with_deploy_code(Bytes::from_hex("0x435f5260205ff3").unwrap());
    let traces: Vec<TraceResults> = provider
        .client()
        .request("trace_callMany", (vec![(WithOtherFields::new(tx), vec![TraceType::Trace])],))
        .await
        .unwrap();

    let latest = provider.get_block_number().await.unwrap();
    assert_eq!(U256::from_be_slice(&traces[0].output), U256::from(latest));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_get_local() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let multicall = Multicall::deploy(&provider).await.unwrap();
    let storage = SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();
    let get_value = Multicall::Call {
        target: *storage.address(),
        callData: storage.getValue().calldata().clone(),
    };
    let nested = Multicall::Call {
        target: *multicall.address(),
        callData: multicall.aggregate(vec![get_value.clone()]).calldata().clone(),
    };
    let receipt = multicall
        .aggregate(vec![get_value, nested])
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let hash = receipt.transaction_hash;

    let traces = provider.trace_transaction(hash).await.unwrap();
    let addresses =
        traces.iter().map(|trace| trace.trace.trace_address.clone()).collect::<Vec<_>>();
    assert_eq!(addresses, vec![vec![], vec![0], vec![1], vec![1, 0]]);

    let trace_get = async |path: &[usize]| {
        let path = path.iter().copied().map(Index::from).collect::<Vec<_>>();
        provider
            .client()
            .request::<_, Option<LocalizedTransactionTrace>>("trace_get", (hash, path))
            .await
            .unwrap()
    };
    for trace in &traces {
        assert_eq!(trace_get(&trace.trace.trace_address).await.as_ref(), Some(trace));
    }
    for missing in [&[2][..], &[0, 0], &[1, 1], &[1, 0, 0]] {
        assert_eq!(trace_get(missing).await, None);
    }

    let unknown = provider
        .client()
        .request::<_, Option<LocalizedTransactionTrace>>(
            "trace_get",
            (B256::ZERO, Vec::<Index>::new()),
        )
        .await
        .unwrap();
    assert_eq!(unknown, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_block_traces_reject_pending() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    api.mine_one().await.unwrap();

    let pending = BlockId::pending();
    let error = provider.trace_block(pending).await.unwrap_err();
    assert_eq!(error.as_error_resp().unwrap().code, -32602);
    let error = provider.trace_replay_block_transactions(pending).await.unwrap_err();
    assert_eq!(error.as_error_resp().unwrap().code, -32602);

    // Mined block tags still resolve.
    provider.trace_block(BlockId::latest()).await.unwrap();
    provider.trace_replay_block_transactions(BlockId::latest()).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_calls_reject_conflicting_fields() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let client = handle.http_provider();
    let client = client.client();
    let from = handle.dev_wallets().next().unwrap().address();
    let input_error = "both \"data\" and \"input\" are set and not equal. Please use \"input\" to \
                       pass transaction call data";
    let fee_error = "both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified";

    for (call, message) in [
        (json!({ "from": from, "to": from, "data": "0x602a", "input": "0x6001" }), input_error),
        (json!({ "from": from, "to": from, "gasPrice": "0x1", "maxFeePerGas": "0x2" }), fee_error),
        (
            json!({ "from": from, "to": from, "gasPrice": "0x1", "maxPriorityFeePerGas": "0x1" }),
            fee_error,
        ),
    ] {
        let simulate = json!({ "blockStateCalls": [{ "calls": [&call] }] });
        let errors = [
            client.request::<_, Value>("eth_call", (&call, "latest")).await.unwrap_err(),
            client.request::<_, Value>("eth_estimateGas", (&call, "latest")).await.unwrap_err(),
            client
                .request::<_, Value>("trace_call", (&call, ["trace"], "latest"))
                .await
                .unwrap_err(),
            client.request::<_, Value>("eth_simulateV1", (&simulate, "latest")).await.unwrap_err(),
            client.request::<_, Value>("eth_sendTransaction", (&call,)).await.unwrap_err(),
        ];
        for error in errors {
            let error = error.as_error_resp().unwrap();
            assert_eq!((error.code, error.message.as_ref()), (-32602, message), "{call}");
        }
    }

    let call = json!({ "from": from, "data": "0x602a", "input": "0x602a" });
    client.request::<_, Value>("eth_call", (&call, "latest")).await.unwrap();
    client.request::<_, Value>("trace_call", (&call, ["trace"], "latest")).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_transaction_omits_nested_precompile_calls() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    // Zero-value CALL to the identity precompile, then a zero-value CALL to 0xbeef.
    let caller = Address::repeat_byte(0x42);
    let code =
        Bytes::from_hex("0x6000600060006000600060045af1506000600060006000600061beef5af15000")
            .unwrap();
    api.anvil_set_code(caller, code).await.unwrap();

    let tx = TransactionRequest::default().from(from).to(caller);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();
    let hash = receipt.transaction_hash;

    let traces = provider.trace_transaction(hash).await.unwrap();
    let traces = traces.into_iter().map(|trace| trace.trace).collect::<Vec<_>>();
    assert_eq!(traces.len(), 2);
    assert_eq!(traces[0].subtraces, 1);
    assert_eq!(traces[1].trace_address, vec![0]);
    let Action::Call(call) = &traces[1].action else { panic!("expected a call") };
    assert_eq!(call.to, Address::left_padding_from(&[0xbe, 0xef]));

    let replay = provider.trace_replay_transaction(hash).trace().await.unwrap();
    assert_eq!(replay.trace, traces);

    let block = provider.trace_block(receipt.block_number.unwrap().into()).await.unwrap();
    assert_eq!(block.into_iter().map(|trace| trace.trace).collect::<Vec<_>>(), traces);

    // Parity filtering must leave both children in the stored Geth call graph.
    let geth = provider
        .debug_trace_transaction(
            hash,
            GethDebugTracingOptions::default()
                .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer)),
        )
        .await
        .unwrap();
    let GethTrace::CallTracer(frame) = geth else { panic!("expected a call trace") };
    assert_eq!(
        frame.calls.iter().map(|call| call.to.unwrap()).collect::<Vec<_>>(),
        [Address::with_last_byte(4), Address::left_padding_from(&[0xbe, 0xef])]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_transaction_keeps_root_and_valued_precompile_calls() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();
    let identity = Address::with_last_byte(4);

    // CALL to the identity precompile forwarding 1 wei.
    let caller = Address::repeat_byte(0x42);
    api.anvil_set_code(caller, Bytes::from_hex("0x6000600060006000600160045af15000").unwrap())
        .await
        .unwrap();
    // DELEGATECALL to the identity precompile, which inherits the frame's value.
    let delegator = Address::repeat_byte(0x43);
    api.anvil_set_code(delegator, Bytes::from_hex("0x600060006000600060045af45000").unwrap())
        .await
        .unwrap();

    let tx = |to, value| TransactionRequest::default().from(from).to(to).value(U256::from(value));
    for (tx, frames) in [
        (tx(caller, 1), vec![caller, identity]),
        (tx(delegator, 1), vec![delegator, identity]),
        (tx(delegator, 0), vec![delegator]),
        (tx(identity, 0).input(Bytes::from_static(b"echo").into()), vec![identity]),
    ] {
        let receipt =
            provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();
        let hash = receipt.transaction_hash;
        let traces = provider.trace_transaction(hash).await.unwrap();
        let traces = traces.into_iter().map(|trace| trace.trace).collect::<Vec<_>>();
        let targets = traces
            .iter()
            .map(|trace| match &trace.action {
                Action::Call(call) => call.to,
                action => panic!("expected a call, got {action:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(targets, frames);

        let replay = provider.trace_replay_transaction(hash).trace().await.unwrap();
        assert_eq!(replay.trace, traces);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mined_precompile_traces_survive_chain_id_changes() {
    let p256 = Address::left_padding_from(&[1, 0]);
    let target = Address::left_padding_from(&[0xbe, 0xef]);
    let caller = Address::repeat_byte(0x42);
    for (chain_id, replacement, expected_targets) in
        [(31_337u64, 56u64, vec![caller, p256, target]), (56u64, 31_337u64, vec![caller, target])]
    {
        let config = NodeConfig::test()
            .with_hardfork(Some(EthereumHardfork::Prague.into()))
            .with_chain_id(Some(chain_id))
            .with_genesis_timestamp(Some(1_718_863_501u64));
        let (api, handle) = spawn(config.clone()).await;
        let provider = handle.http_provider();
        let from = handle.dev_wallets().next().unwrap().address();

        // STATICCALL to 0x0100, then CALL to 0xbeef. BSC installs P256 after Haber.
        api.anvil_set_code(
            caller,
            Bytes::from_hex("0x60006000600060006101005afa506000600060006000600061beef5af15000")
                .unwrap(),
        )
        .await
        .unwrap();
        let receipt = provider
            .send_transaction(WithOtherFields::new(
                TransactionRequest::default().from(from).to(caller),
            ))
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        let hash = receipt.transaction_hash;
        let block = receipt.block_number.unwrap();
        let before = provider.trace_transaction(hash).await.unwrap();
        let targets = before
            .iter()
            .map(|trace| match &trace.trace.action {
                Action::Call(call) => call.to,
                action => panic!("expected a call, got {action:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(targets, expected_targets, "chain ID {chain_id}");
        assert_eq!(before[0].trace.subtraces, before.len() - 1);
        for (index, trace) in before[1..].iter().enumerate() {
            assert_eq!(trace.trace.trace_address, vec![index]);
        }

        api.anvil_set_chain_id(replacement).await.unwrap();
        assert_eq!(provider.trace_transaction(hash).await.unwrap(), before);
        assert_eq!(provider.trace_block(block.into()).await.unwrap(), before);
        assert_eq!(
            provider
                .trace_filter(&TraceFilter::default().from_block(block).to_block(block))
                .await
                .unwrap(),
            before
        );

        // The execution marker must also survive serialization into a fresh node.
        let state = api.anvil_dump_state(None).await.unwrap();
        let (restored_api, restored_handle) = spawn(config.with_chain_id(Some(replacement))).await;
        assert!(restored_api.anvil_load_state(state).await.unwrap());
        assert_eq!(restored_handle.http_provider().trace_transaction(hash).await.unwrap(), before);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_transaction_unknown_hash_local() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let traces = handle
        .http_provider()
        .client()
        .request::<_, Option<Vec<LocalizedTransactionTrace>>>("trace_transaction", (B256::ZERO,))
        .await
        .unwrap();
    assert_eq!(traces, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_transaction_unknown_hash_fork() {
    let (origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin = origin_handle.http_provider();
    origin_api.mine_one().await.unwrap();
    origin_api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = origin_handle.dev_wallets().collect::<Vec<_>>();
    let tx = TransactionRequest::default()
        .to(accounts[1].address())
        .value(U256::from(1000))
        .from(accounts[0].address());
    let tx = WithOtherFields::new(tx);
    let hash = *origin.send_transaction(tx).await.unwrap().tx_hash();

    let config = NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    for hash in [B256::ZERO, hash] {
        let traces = provider
            .client()
            .request::<_, Option<Vec<LocalizedTransactionTrace>>>("trace_transaction", (hash,))
            .await
            .unwrap();
        assert_eq!(traces, None);
    }

    // A missing hash is not cached, so it resolves once mined upstream.
    origin_api.mine_one().await.unwrap();
    let traces = provider.trace_transaction(hash).await.unwrap();
    assert!(!traces.is_empty());
    assert_eq!(traces, origin.trace_transaction(hash).await.unwrap());
}

sol!(
    #[sol(rpc, bytecode = "0x6080604052348015600f57600080fd5b50336000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff16021790555060a48061005e6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c806375fc8e3c14602d575b600080fd5b60336035565b005b60008054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16fffea26469706673582212205006867290df97c54f2df1cb94fc081197ab670e2adf5353071d2ecce1d694b864736f6c634300080d0033")]
    contract SuicideContract {
        address payable private owner;
        constructor() public {
            owner = payable(msg.sender);
        }
        function goodbye() public {
            selfdestruct(owner);
        }
    }
);

#[tokio::test(flavor = "multi_thread")]
async fn test_parity_suicide_trace() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Shanghai.into()))).await;
    let provider = handle.ws_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let owner = wallets[0].address();
    let destructor = wallets[1].address();

    let contract_addr =
        SuicideContract::deploy_builder(provider.clone()).from(owner).deploy().await.unwrap();
    let contract = SuicideContract::new(contract_addr, provider.clone());
    let call = contract.goodbye().from(destructor);
    let call = call.send().await.unwrap();
    let tx = call.get_receipt().await.unwrap();

    let traces = handle.http_provider().trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    assert!(traces[1].trace.action.is_selfdestruct());
}

sol!(
    #[sol(rpc, bytecode = "0x6080604052348015600f57600080fd5b50336000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff16021790555060a48061005e6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c806375fc8e3c14602d575b600080fd5b60336035565b005b60008054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16fffea26469706673582212205006867290df97c54f2df1cb94fc081197ab670e2adf5353071d2ecce1d694b864736f6c634300080d0033")]
    contract DebugTraceContract {
        address payable private owner;
        constructor() public {
            owner = payable(msg.sender);
        }
        function goodbye() public {
            selfdestruct(owner);
        }
    }
);

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_debug_trace_call() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let contract_addr = DebugTraceContract::deploy_builder(provider.clone())
        .from(wallets[0].clone().address())
        .deploy()
        .await
        .unwrap();

    let caller: EthereumWallet = wallets[1].clone().into();
    let caller_provider = http_provider_with_signer(&handle.http_endpoint(), caller);
    let contract = DebugTraceContract::new(contract_addr, caller_provider);

    let call = contract.goodbye().from(wallets[1].address());
    let calldata = call.calldata().to_owned();

    let tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*contract.address())
        .with_input(calldata);

    let traces = handle
        .http_provider()
        .debug_trace_call(
            WithOtherFields::new(tx),
            BlockId::latest(),
            GethDebugTracingCallOptions::default(),
        )
        .await
        .unwrap();

    match traces {
        GethTrace::Default(default_frame) => {
            assert!(!default_frame.failed);
        }
        _ => {
            unreachable!()
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_tracer_debug_trace_call() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let multicall_contract = Multicall::deploy(&provider).await.unwrap();

    let simple_storage_contract =
        SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();

    let set_value = simple_storage_contract.setValue("bar".to_string());
    let set_value_calldata = set_value.calldata();

    let internal_call_tx_builder = multicall_contract.aggregate(vec![Multicall::Call {
        target: *simple_storage_contract.address(),
        callData: set_value_calldata.to_owned(),
    }]);

    let internal_call_tx_calldata = internal_call_tx_builder.calldata().to_owned();

    // calling SimpleStorage contract through Multicall should result in an internal call
    let internal_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*multicall_contract.address())
        .with_input(internal_call_tx_calldata);

    let internal_call_tx_traces = handle
        .http_provider()
        .debug_trace_call(
            WithOtherFields::new(internal_call_tx.clone()),
            BlockId::latest(),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log()),
            ),
        )
        .await
        .unwrap();

    match internal_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert!(call_frame.calls.len() == 1);
            assert!(
                call_frame.calls.first().unwrap().to.unwrap() == *simple_storage_contract.address()
            );
            assert!(call_frame.calls.first().unwrap().logs.len() == 1);
        }
        _ => {
            unreachable!()
        }
    }

    // only_top_call option - should not return any internal calls
    let internal_call_only_top_call_tx_traces = handle
        .http_provider()
        .debug_trace_call(
            WithOtherFields::new(internal_call_tx.clone()),
            BlockId::latest(),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log().only_top_call()),
            ),
        )
        .await
        .unwrap();

    match internal_call_only_top_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert!(call_frame.calls.is_empty());
        }
        _ => {
            unreachable!()
        }
    }

    let receipt = internal_call_tx_builder.send().await.unwrap().get_receipt().await.unwrap();
    let internal_call_tx_hash = receipt.transaction_hash;
    let trace_provider = handle.http_provider();

    let internal_call_tx_traces: GethTrace = trace_provider
        .raw_request(
            "debug_traceTransaction".into(),
            (
                internal_call_tx_hash,
                serde_json::json!({
                    "tracer": "callTracer",
                    "tracerConfig": {
                        "withLog": true
                    }
                }),
            ),
        )
        .await
        .unwrap();

    match internal_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert_eq!(call_frame.calls.len(), 1);
            assert_eq!(
                call_frame.calls.first().unwrap().to.unwrap(),
                *simple_storage_contract.address()
            );
        }
        _ => {
            unreachable!()
        }
    }

    let internal_call_only_top_level_call_tx_traces: GethTrace = trace_provider
        .raw_request(
            "debug_traceTransaction".into(),
            (
                internal_call_tx_hash,
                serde_json::json!({
                    "tracer": "callTracer",
                    "tracerConfig": {
                        "onlyTopLevelCall": true,
                        "withLog": true
                    }
                }),
            ),
        )
        .await
        .unwrap();

    match internal_call_only_top_level_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert!(call_frame.calls.is_empty());
        }
        _ => {
            unreachable!()
        }
    }

    // directly calling the SimpleStorage contract should not result in any internal calls
    let direct_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*simple_storage_contract.address())
        .with_input(set_value_calldata.to_owned());

    let direct_call_tx_traces = handle
        .http_provider()
        .debug_trace_call(
            WithOtherFields::new(direct_call_tx),
            BlockId::latest(),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log()),
            ),
        )
        .await
        .unwrap();

    match direct_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert!(call_frame.calls.is_empty());
            assert!(call_frame.to.unwrap() == *simple_storage_contract.address());
            assert!(call_frame.logs.len() == 1);
        }
        _ => {
            unreachable!()
        }
    }
    api.anvil_set_auto_mine(false).await.unwrap();
    let nonce = provider.get_transaction_count(wallets[1].address()).await.unwrap();
    let mut hashes = Vec::new();
    for nonce in nonce..nonce + 2 {
        hashes.push(
            api.send_transaction(WithOtherFields::new(
                internal_call_tx.clone().nonce(nonce).gas_limit(500_000),
            ))
            .await
            .unwrap(),
        );
    }
    api.mine_one().await.unwrap();
    let block_number = provider.get_block_number().await.unwrap();
    for config in [
        serde_json::json!({"withLog": true}),
        serde_json::json!({"withLog": true, "onlyTopCall": true}),
        serde_json::json!({"withLog": true, "onlyTopLevelCall": true}),
    ] {
        let options = serde_json::from_value::<GethDebugTracingOptions>(serde_json::json!({
            "tracer": "callTracer", "tracerConfig": config,
        }))
        .unwrap();
        let mut expected = Vec::new();
        for hash in &hashes {
            let result = api.backend.debug_trace_transaction(*hash, options.clone()).await.unwrap();
            expected.push(TraceResult::Success { result, tx_hash: Some(*hash) });
        }
        assert_eq!(
            api.backend.debug_trace_block_by_number(block_number.into(), options).await.unwrap(),
            expected,
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_call_state_override() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(address!("0x1234567890123456789012345678901234567890"));

    let override_json = r#"{
            "0x1234567890123456789012345678901234567890": {
                "balance": "0x01",
                "code": "0x30315f5260205ff3"
            }
        }"#;

    let state_override: StateOverride = serde_json::from_str(override_json).unwrap();

    let tx_traces = handle
        .http_provider()
        .debug_trace_call(
            WithOtherFields::new(tx.clone()),
            BlockId::latest(),
            GethDebugTracingCallOptions::default()
                .with_tracing_options(GethDebugTracingOptions::default())
                .with_state_overrides(state_override),
        )
        .await
        .unwrap();

    match tx_traces {
        GethTrace::Default(trace_res) => {
            assert_eq!(
                trace_res.return_value,
                Bytes::from_hex("0000000000000000000000000000000000000000000000000000000000000001")
                    .unwrap()
            );
        }
        _ => {
            unreachable!()
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_call_tx_index() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let simple_storage_contract =
        SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();

    api.anvil_set_auto_mine(false).await.unwrap();

    let from = wallets[0].address();
    let nonce = provider.get_transaction_count(from).await.unwrap();
    for (offset, value) in ["first", "second", "third"].into_iter().enumerate() {
        let set_value = simple_storage_contract.setValue(value.to_string());
        let tx = TransactionRequest::default()
            .from(from)
            .to(*simple_storage_contract.address())
            .with_input(set_value.calldata().to_owned())
            .nonce(nonce + offset as u64);
        let _ = provider.send_transaction(WithOtherFields::new(tx)).await.unwrap();
    }

    api.mine_one().await.unwrap();
    let block_number = provider.get_block_number().await.unwrap();

    let get_value = simple_storage_contract.getValue();
    let call = TransactionRequest::default()
        .from(from)
        .to(*simple_storage_contract.address())
        .with_input(get_value.calldata().to_owned());

    for (tx_index, expected) in [(0, "init value"), (1, "first"), (2, "second")] {
        let trace = provider
            .debug_trace_call(
                WithOtherFields::new(call.clone()),
                BlockId::number(block_number),
                GethDebugTracingCallOptions::default().with_tx_index(tx_index),
            )
            .await
            .unwrap();
        match trace {
            GethTrace::Default(default_frame) => {
                assert_eq!(
                    default_frame.return_value,
                    Bytes::from(String::from(expected).abi_encode())
                );
            }
            _ => unreachable!(),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_transaction_reports_transaction_gas() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let nonce = provider.get_transaction_count(from).await.unwrap();

    let first = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(from).to(to).value(U256::ONE).nonce(nonce),
        ))
        .await
        .unwrap();
    let second = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(from).to(to).value(U256::from(2)).nonce(nonce + 1),
        ))
        .await
        .unwrap();

    api.mine_one().await.unwrap();
    let first_receipt = first.get_receipt().await.unwrap();
    let second_receipt = second.get_receipt().await.unwrap();
    assert_eq!(first_receipt.block_hash, second_receipt.block_hash);

    let default_trace = api
        .debug_trace_transaction(
            second_receipt.transaction_hash,
            GethDebugTracingOptions::default(),
        )
        .await
        .unwrap();
    let GethTrace::Default(default_frame) = default_trace else {
        unreachable!("expected default trace")
    };
    assert_eq!(default_frame.gas, second_receipt.gas_used);

    let call_trace = api
        .debug_trace_transaction(
            second_receipt.transaction_hash,
            GethDebugTracingOptions::default()
                .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer)),
        )
        .await
        .unwrap();
    let GethTrace::CallTracer(call_frame) = call_trace else { unreachable!("expected call trace") };
    assert_eq!(call_frame.gas_used, U256::from(second_receipt.gas_used));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_transaction_struct_logs_with_steps_tracing() {
    let (api, handle) = spawn(NodeConfig::test().with_steps_tracing(true)).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let nonce = provider.get_transaction_count(from).await.unwrap();

    // Precede the traced transaction so that tracing replays the block prefix.
    let transfer = TransactionRequest::default()
        .from(from)
        .to(accounts[1].address())
        .value(U256::ONE)
        .nonce(nonce);
    let _ = provider.send_transaction(WithOtherFields::new(transfer)).await.unwrap();

    // PUSH1 0x2a PUSH1 0x00 MSTORE PUSH1 0x20 PUSH1 0x00 RETURN
    let init_code = Bytes::from_hex("602a60005260206000f3").unwrap();
    let create = TransactionRequest::default()
        .from(from)
        .with_deploy_code(init_code)
        .nonce(nonce + 1)
        .gas_limit(100_000);
    let pending = provider.send_transaction(WithOtherFields::new(create)).await.unwrap();

    api.mine_one().await.unwrap();
    let receipt = pending.get_receipt().await.unwrap();
    assert_eq!(receipt.transaction_index, Some(1));

    let word = "0x000000000000000000000000000000000000000000000000000000000000002a".to_string();
    for memory in [false, true] {
        let config = GethDefaultTracingOptions::default().with_enable_memory(memory);
        let opts = GethDebugTracingOptions { config, ..Default::default() };
        let trace = provider.debug_trace_transaction(receipt.transaction_hash, opts).await.unwrap();
        let GethTrace::Default(frame) = trace else { unreachable!("expected default trace") };
        assert_eq!(frame.gas, receipt.gas_used);
        assert!(!frame.failed);

        let steps = frame
            .struct_logs
            .into_iter()
            .map(|log| (log.pc, log.op.into_owned(), log.stack.unwrap(), log.memory))
            .collect::<Vec<_>>();
        let empty = memory.then(Vec::new);
        let stored = memory.then(|| vec![word.clone()]);
        assert_eq!(
            steps,
            [
                (0, "PUSH1".to_string(), vec![], empty.clone()),
                (2, "PUSH1".to_string(), vec![U256::from(0x2a)], empty.clone()),
                (4, "MSTORE".to_string(), vec![U256::from(0x2a), U256::ZERO], empty),
                (5, "PUSH1".to_string(), vec![], stored.clone()),
                (7, "PUSH1".to_string(), vec![U256::from(0x20)], stored.clone()),
                (9, "RETURN".to_string(), vec![U256::from(0x20), U256::ZERO], stored),
            ]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_transaction_replays_blob_base_fee() {
    let (api, handle) = spawn(NodeConfig::test().with_steps_tracing(true)).await;
    let provider = handle.http_provider();

    let blob_update_fraction = u64::try_from(api.backend.blob_params().update_fraction).unwrap();
    api.backend.fees().set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
        30_000_000,
        blob_update_fraction,
    ));

    // BLOBBASEFEE PUSH0 MSTORE PUSH1 0x20 PUSH0 RETURN
    let init_code = Bytes::from_hex("4a5f5260205ff3").unwrap();
    let from = handle.dev_wallets().next().unwrap().address();
    let create = TransactionRequest::default().from(from).with_deploy_code(init_code);
    let receipt = provider
        .send_transaction(WithOtherFields::new(create))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // The deployed code is the blob base fee observed while mining.
    let code = provider.get_code_at(receipt.contract_address.unwrap()).await.unwrap();
    let mined_blob_base_fee = U256::from_be_slice(&code);
    assert!(mined_blob_base_fee > U256::ONE);

    let trace = provider
        .debug_trace_transaction(receipt.transaction_hash, GethDebugTracingOptions::default())
        .await
        .unwrap();
    let GethTrace::Default(frame) = trace else { unreachable!("expected default trace") };
    assert_eq!(frame.struct_logs[1].op, "PUSH0");
    assert_eq!(frame.struct_logs[1].stack, Some(vec![mined_blob_base_fee]));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_replays_report_unavailable_historical_state() {
    let (api, handle) = spawn(NodeConfig::test().with_steps_tracing(true)).await;
    let from = handle.dev_wallets().next().unwrap().address();
    let tx = TransactionRequest::default().from(from).to(from).value(U256::ONE);
    let receipt = handle
        .http_provider()
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // Restoring without historical states keeps the transaction but not its parent state.
    let state = api.serialized_state(false).await.unwrap();
    let (api, handle) =
        spawn(NodeConfig::test().with_steps_tracing(true).with_init_state(Some(state))).await;
    let provider = handle.http_provider();
    let message = "historical state needed to replay block 1 is not available";

    let call_tracer = GethDebugTracingOptions::default()
        .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer));
    for opts in [GethDebugTracingOptions::default(), call_tracer] {
        let error =
            provider.debug_trace_transaction(receipt.transaction_hash, opts).await.unwrap_err();
        let error = error.as_error_resp().unwrap();
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, message);
    }

    let error = api
        .trace_replay_block_transactions(
            BlockNumberOrTag::Number(1),
            [TraceType::Trace].into_iter().collect(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);

    // Genesis has no transactions to replay.
    let genesis = api
        .trace_replay_block_transactions(
            BlockNumberOrTag::Number(0),
            [TraceType::Trace].into_iter().collect(),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(genesis.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_unknown_block_and_transaction() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let head = provider.get_block_number().await.unwrap();
    let next = BlockId::number(head + 1);

    let error = provider.trace_block(next).await.unwrap_err();
    assert_eq!(error.as_error_resp().unwrap().code, -32001);
    let replays = provider
        .client()
        .request::<_, Option<Vec<TraceResultsWithTransactionHash>>>(
            "trace_replayBlockTransactions",
            (next, vec![TraceType::Trace]),
        )
        .await
        .unwrap();
    assert_eq!(replays, None);

    let replay = provider
        .client()
        .request::<_, Option<TraceResults>>(
            "trace_replayTransaction",
            (B256::ZERO, vec![TraceType::Trace]),
        )
        .await
        .unwrap();
    assert_eq!(replay, None);

    let filter = TraceFilter::default().from_block(head).to_block(head + 1);
    let error = provider.trace_filter(&filter).await.unwrap_err();
    assert_eq!(error.as_error_resp().unwrap().code, -32001);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_fork_block() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let accounts = origin_handle.dev_wallets().collect::<Vec<_>>();
    let tx = WithOtherFields::new(
        TransactionRequest::default()
            .to(accounts[1].address())
            .value(U256::from(1))
            .from(accounts[0].address()),
    );
    origin_handle
        .http_provider()
        .send_transaction(tx.clone())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let (_api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()))).await;
    let provider = handle.http_provider();
    let fork_block = provider.get_block_number().await.unwrap();

    // The fork block is not stored locally, so its traces come from the fork.
    assert_eq!(provider.trace_block(BlockId::latest()).await.unwrap().len(), 1);
    assert_eq!(provider.trace_replay_block_transactions(BlockId::latest()).await.unwrap().len(), 1);

    provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let filter = TraceFilter::default().from_block(fork_block).to_block(fork_block + 1);
    let blocks = provider
        .trace_filter(&filter)
        .await
        .unwrap()
        .into_iter()
        .map(|trace| trace.block_number.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(blocks, [fork_block, fork_block + 1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_transaction_rejects_unknown_hash() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let error = handle
        .http_provider()
        .debug_trace_transaction(B256::ZERO, GethDebugTracingOptions::default())
        .await
        .unwrap_err();
    let error = error.as_error_resp().unwrap();

    assert_eq!(error.code, -32001);
    assert_eq!(error.message, "transaction not found");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_call_tx_index_fork_lazy_state() {
    let (_api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    let tx = TransactionRequest::default().from(from).to(Address::random()).value(U256::ONE);
    let _ = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let block_number = provider.get_block_number().await.unwrap();

    let usdc = address!("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
    let total_supply = TransactionRequest::default()
        .from(from)
        .to(usdc)
        .with_input(Bytes::from_hex("18160ddd").unwrap());

    let tx_index_trace = provider
        .debug_trace_call(
            WithOtherFields::new(total_supply.clone()),
            BlockId::number(block_number),
            GethDebugTracingCallOptions::default().with_tx_index(0),
        )
        .await
        .unwrap();
    let GethTrace::Default(tx_index_frame) = tx_index_trace else { unreachable!() };

    let latest_trace = provider
        .debug_trace_call(
            WithOtherFields::new(total_supply),
            BlockId::number(block_number),
            GethDebugTracingCallOptions::default(),
        )
        .await
        .unwrap();
    let GethTrace::Default(latest_frame) = latest_trace else { unreachable!() };

    assert!(!tx_index_frame.return_value.is_empty());
    assert_eq!(tx_index_frame.return_value, latest_frame.return_value);
}

// <https://github.com/foundry-rs/foundry/issues/2656>
#[tokio::test(flavor = "multi_thread")]
async fn test_trace_address_fork() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15291050u64))).await;
    let provider = handle.http_provider();

    let input = hex::decode("43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67").unwrap();

    let from = address!("0x2e4777139254ff76db957e284b186a4507ff8c67");
    let to = address!("0xe2f2a5c287993345a840db3b0845fbc70f5935a5");
    let tx = TransactionRequest::default()
        .to(to)
        .from(from)
        .with_input::<Bytes>(input.into())
        .with_gas_limit(300_000);

    let tx = WithOtherFields::new(tx);
    api.anvil_impersonate_account(from).await.unwrap();

    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    match traces[0].trace.action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
        }
        _ => unreachable!("unexpected action"),
    }

    let json = serde_json::json!([
        {
            "action": {
                "callType": "call",
                "from": "0x2e4777139254ff76db957e284b186a4507ff8c67",
                "gas": "0x262b3",
                "input": "0x43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67",
                "to": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x2131b",
                "output": "0x0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3"
            },
            "subtraces": 1,
            "traceAddress": [],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "delegatecall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0x23d88",
                "input": "0x43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67",
                "to": "0x15b2838cd28cc353afbe59385db3f366d8945aee",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x1f6e1",
                "output": "0x0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3"
            },
            "subtraces": 2,
            "traceAddress": [0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "staticcall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0x192ed",
                "input": "0x50494dc000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000e0b1ff65617f654b2f00000000000000000000000000000000000000000000000000000000000061a800000000000000000000000000000000000000000000000000b1a2bc2ec5000000000000000000000000000000000000000000000000000006f05b59d3b2000000000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000005f5e1000000000000000000000000000000000000000000000414ec22973db48fd3a3370000000000000000000000000000000000000000000000056bc75e2d6310000000000000000000000000000000000000000000000000000000000a7314a9ba5c0000000000000000000000000000000000000000000000000000000005f5e100000000000000000000000000000000000000000000095f783edc5a5dabcb4ba70000000000000000000000000000000000000000000000056bc75e2d6310000000000000000000000000000000000000000000000000000000000a3f42df4dab",
                "to": "0xca480d596e6717c95a62a4dc1bd4fbd7b7e7d705",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x661a",
                "output": "0x0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3"
            },
            "subtraces": 0,
            "traceAddress": [0, 0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "delegatecall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0xd2dc",
                "input": "0x4e331a540000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d30000000000000000000000006b175474e89094c44da98b954eedeac495271d0f000000000000000000000000a2a3cae63476891ab2d640d9a5a800755ee79d6e000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000005f5e100000000000000000000000000000000000000000000095f783edc5a5dabcb4ba70000000000000000000000002e4777139254ff76db957e284b186a4507ff8c6700000000000000000000000000000000000000000000f7be2b91f8a2e2df496e",
                "to": "0x1e91f826fa8aa4fa4d3f595898af3a64dd188848",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x7617",
                "output": "0x"
            },
            "subtraces": 2,
            "traceAddress": [0, 1],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "staticcall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0xbf50",
                "input": "0x70a08231000000000000000000000000a2a3cae63476891ab2d640d9a5a800755ee79d6e",
                "to": "0x6b175474e89094c44da98b954eedeac495271d0f",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0xa2a",
                "output": "0x0000000000000000000000000000000000000000000020fe99f8898600d94750"
            },
            "subtraces": 0,
            "traceAddress": [0, 1, 0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "call",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0xa92a",
                "input": "0xa4e285950000000000000000000000002e4777139254ff76db957e284b186a4507ff8c670000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3",
                "to": "0xa2a3cae63476891ab2d640d9a5a800755ee79d6e",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x4ed3",
                "output": "0x"
            },
            "subtraces": 1,
            "traceAddress": [0, 1, 1],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "call",
                "from": "0xa2a3cae63476891ab2d640d9a5a800755ee79d6e",
                "gas": "0x8c90",
                "input": "0xa9059cbb0000000000000000000000002e4777139254ff76db957e284b186a4507ff8c670000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3",
                "to": "0x6b175474e89094c44da98b954eedeac495271d0f",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x2b42",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 1, 1, 0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        }
    ]);

    let expected_traces: Vec<LocalizedTransactionTrace> = serde_json::from_value(json).unwrap();

    // test matching traceAddress
    traces.into_iter().zip(expected_traces).for_each(|(a, b)| {
        assert_eq!(a.trace.trace_address, b.trace.trace_address);
        assert_eq!(a.trace.subtraces, b.trace.subtraces);
        match (a.trace.action, b.trace.action) {
            (Action::Call(a), Action::Call(b)) => {
                assert_eq!(a.from, b.from);
                assert_eq!(a.to, b.to);
            }
            _ => unreachable!("unexpected action"),
        }
    })
}

// <https://github.com/foundry-rs/foundry/issues/2705>
// <https://etherscan.io/tx/0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845>
#[tokio::test(flavor = "multi_thread")]
async fn test_trace_address_fork2() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15314401u64))).await;
    let provider = handle.http_provider();

    let input = hex::decode("30000003000000000000000000000000adda1059a6c6c102b0fa562b9bb2cb9a0de5b1f4000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a300000004fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000004319b52bf08b65295d49117e790000000000000000000000000000000000000000000000008b6d9e8818d6141f000000000000000000000000000000000000000000000000000000086a23af210000000000000000000000000000000000000000000000000000000000").unwrap();

    let from = address!("0xa009fa1ac416ec02f6f902a3a4a584b092ae6123");
    let to = address!("0x99999999d116ffa7d76590de2f427d8e15aeb0b8");
    let tx = TransactionRequest::default()
        .to(to)
        .from(from)
        .with_input::<Bytes>(input.into())
        .with_gas_limit(350_000);

    let tx = WithOtherFields::new(tx);
    api.anvil_impersonate_account(from).await.unwrap();

    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();

    assert!(!traces.is_empty());
    match traces[0].trace.action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
        }
        _ => unreachable!("unexpected action"),
    }

    let json = serde_json::json!([
        {
            "action": {
                "from": "0xa009fa1ac416ec02f6f902a3a4a584b092ae6123",
                "callType": "call",
                "gas": "0x4fabc",
                "input": "0x30000003000000000000000000000000adda1059a6c6c102b0fa562b9bb2cb9a0de5b1f4000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a300000004fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000004319b52bf08b65295d49117e790000000000000000000000000000000000000000000000008b6d9e8818d6141f000000000000000000000000000000000000000000000000000000086a23af210000000000000000000000000000000000000000000000000000000000",
                "to": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x1d51b",
                "output": "0x"
            },
            "subtraces": 1,
            "traceAddress": [],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "delegatecall",
                "gas": "0x4d594",
                "input": "0x00000004fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000004319b52bf08b65295d49117e790000000000000000000000000000000000000000000000008b6d9e8818d6141f000000000000000000000000000000000000000000000000000000086a23af21",
                "to": "0xadda1059a6c6c102b0fa562b9bb2cb9a0de5b1f4",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x1c35f",
                "output": "0x"
            },
            "subtraces": 3,
            "traceAddress": [0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "call",
                "gas": "0x4b6d6",
                "input": "0x16b2da82000000000000000000000000000000000000000000000000000000086a23af21",
                "to": "0xd1663cfb8ceaf22039ebb98914a8c98264643710",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0xd6d",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000000"
            },
            "subtraces": 0,
            "traceAddress": [0, 0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "staticcall",
                "gas": "0x49c35",
                "input": "0x3850c7bd",
                "to": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0xa88",
                "output": "0x000000000000000000000000000000000000004319b52bf08b65295d49117e7900000000000000000000000000000000000000000000000000000000000148a0000000000000000000000000000000000000000000000000000000000000010e000000000000000000000000000000000000000000000000000000000000012c000000000000000000000000000000000000000000000000000000000000012c00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 1],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "call",
                "gas": "0x48d01",
                "input": "0x128acb0800000000000000000000000099999999d116ffa7d76590de2f427d8e15aeb0b80000000000000000000000000000000000000000000000000000000000000001fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb98000000000000000000000000000000000000000000000000000000001000276a400000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000002bc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000000000000000000000000000000000",
                "to": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x18c20",
                "output": "0x0000000000000000000000000000000000000000000000008b5116525f9edc3efffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980"
            },
            "subtraces": 4,
            "traceAddress": [0, 2],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "call",
                "gas": "0x3802a",
                "input": "0xa9059cbb00000000000000000000000099999999d116ffa7d76590de2f427d8e15aeb0b8000000000000000000000000000000000000000000000986236e1301eaf04680",
                "to": "0xf4d2888d29d722226fafa5d9b24f9164c092421e",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x31b6",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "staticcall",
                "gas": "0x34237",
                "input": "0x70a082310000000000000000000000004b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "to": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x9e6",
                "output": "0x000000000000000000000000000000000000000000000091cda6c1ce33e53b89"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 1],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "call",
                "gas": "0x3357e",
                "input": "0xfa461e330000000000000000000000000000000000000000000000008b5116525f9edc3efffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb9800000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000002bc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000000000000000000000000000000000",
                "to": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x2e8b",
                "output": "0x"
            },
            "subtraces": 1,
            "traceAddress": [0, 2, 2],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "call",
                "gas": "0x324db",
                "input": "0xa9059cbb0000000000000000000000004b5ab61593a2401b1075b90c04cbcdd3f87ce0110000000000000000000000000000000000000000000000008b5116525f9edc3e",
                "to": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x2a6e",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 2, 0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "staticcall",
                "gas": "0x30535",
                "input": "0x70a082310000000000000000000000004b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "to": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x216",
                "output": "0x00000000000000000000000000000000000000000000009258f7d820938417c7"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 3],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        }
    ]);

    let expected_traces: Vec<LocalizedTransactionTrace> = serde_json::from_value(json).unwrap();

    // test matching traceAddress
    traces.into_iter().zip(expected_traces).for_each(|(a, b)| {
        assert_eq!(a.trace.trace_address, b.trace.trace_address);
        assert_eq!(a.trace.subtraces, b.trace.subtraces);
        match (a.trace.action, b.trace.action) {
            (Action::Call(a), Action::Call(b)) => {
                assert_eq!(a.from, b.from);
                assert_eq!(a.to, b.to);
            }
            _ => unreachable!("unexpected action"),
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_get_rejects_integer_indices() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let tx = TransactionRequest::default()
        .to(accounts[1].address())
        .value(U256::from(1))
        .from(accounts[0].address());
    let receipt = provider
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let hash = receipt.transaction_hash;

    let error = provider
        .client()
        .request::<_, Option<LocalizedTransactionTrace>>("trace_get", (hash, [0]))
        .await
        .unwrap_err();
    assert_eq!(error.as_error_resp().unwrap().code, -32602);

    // Quantity strings are accepted.
    provider
        .client()
        .request::<_, Option<LocalizedTransactionTrace>>("trace_get", (hash, ["0x0"]))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_filter() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let from_two = accounts[2].address();
    let to_two = accounts[3].address();

    // Test default block ranges: omitted bounds both default to latest.
    let tracer = TraceFilter {
        from_block: None,
        to_block: None,
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Intersection,
        after: None,
        count: None,
    };

    for i in 0..=5 {
        let tx = TransactionRequest::default().to(to).value(U256::from(i)).from(from);
        let tx = WithOtherFields::new(tx);
        provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    }

    let latest = provider.get_block_number().await.unwrap();
    let traces = api.trace_filter(tracer.clone()).await.unwrap();
    assert_eq!(traces.len(), 1);
    assert_eq!(traces[0].block_number, Some(latest));

    let traces =
        api.trace_filter(TraceFilter { from_block: Some(0), ..tracer.clone() }).await.unwrap();
    assert_eq!(traces.len(), 6);

    // An explicit end before the implicit latest start is a reversed range.
    let traces = api.trace_filter(TraceFilter { to_block: Some(1), ..tracer }).await;
    assert!(traces.is_err());

    // Test filtering by address
    let tracer = TraceFilter {
        from_block: Some(provider.get_block_number().await.unwrap()),
        to_block: None,
        from_address: vec![from_two],
        to_address: vec![to_two],
        mode: TraceFilterMode::Intersection,
        after: None,
        count: None,
    };

    for i in 0..=5 {
        let tx = TransactionRequest::default().to(to).value(U256::from(i)).from(from);
        let tx = WithOtherFields::new(tx);
        provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

        let tx = TransactionRequest::default().to(to_two).value(U256::from(i)).from(from_two);
        let tx = WithOtherFields::new(tx);
        provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    }

    let traces = api.trace_filter(tracer).await.unwrap();
    assert_eq!(traces.len(), 6);

    // Test for the following actions:
    // Create (deploy the contract)
    // Call (goodbye function)
    // SelfDestruct (side-effect of goodbye)
    let contract_addr =
        SuicideContract::deploy_builder(provider.clone()).from(from).deploy().await.unwrap();
    let contract = SuicideContract::new(contract_addr, provider.clone());

    // Test TraceActions
    let tracer = TraceFilter {
        from_block: Some(provider.get_block_number().await.unwrap()),
        to_block: None,
        from_address: vec![from, contract_addr],
        to_address: vec![], // Leave as 0 address
        mode: TraceFilterMode::Union,
        after: None,
        count: None,
    };

    // Execute call
    let call = contract.goodbye().from(from);
    let call = call.send().await.unwrap();
    call.get_receipt().await.unwrap();

    // Mine transactions to filter against
    for i in 0..=5 {
        let tx = TransactionRequest::default().to(to_two).value(U256::from(i)).from(from_two);
        let tx = WithOtherFields::new(tx);
        provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    }

    let traces = api.trace_filter(tracer).await.unwrap();
    assert_eq!(traces.len(), 3);

    // Test Range Error
    let latest = provider.get_block_number().await.unwrap();
    let tracer = TraceFilter {
        from_block: Some(latest),
        to_block: Some(latest + 301),
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Union,
        after: None,
        count: None,
    };

    let traces = api.trace_filter(tracer).await;
    assert!(traces.is_err());

    // Test same from and to block is valid
    let latest = provider.get_block_number().await.unwrap();
    let tracer = TraceFilter {
        from_block: Some(latest),
        to_block: Some(latest),
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Union,
        after: None,
        count: None,
    };

    let traces = api.trace_filter(tracer).await;
    assert!(traces.is_ok());

    // Test invalid block range
    let latest = provider.get_block_number().await.unwrap();
    let tracer = TraceFilter {
        from_block: Some(latest + 10),
        to_block: Some(latest),
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Union,
        after: None,
        count: None,
    };

    let traces = api.trace_filter(tracer).await;
    assert!(traces.is_err());

    // Test after and count
    let tracer = TraceFilter {
        from_block: Some(provider.get_block_number().await.unwrap()),
        to_block: None,
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Union,
        after: Some(3),
        count: Some(5),
    };

    for i in 0..=10 {
        let tx = TransactionRequest::default().to(to).value(U256::from(i)).from(from);
        let tx = WithOtherFields::new(tx);
        provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    }

    let traces = api.trace_filter(tracer).await.unwrap();
    assert_eq!(traces.len(), 5);
}

#[cfg(feature = "js-tracer")]
#[tokio::test(flavor = "multi_thread")]
async fn test_call_tracer_debug_trace_call_js_tracer() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let multicall_contract = Multicall::deploy(&provider).await.unwrap();
    let simple_storage_contract =
        SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();

    let set_value = simple_storage_contract.setValue("bar".to_string());
    let set_value_calldata = set_value.calldata();

    let internal_call_tx_builder = multicall_contract.aggregate(vec![Multicall::Call {
        target: *simple_storage_contract.address(),
        callData: set_value_calldata.to_owned(),
    }]);

    let internal_call_tx_calldata = internal_call_tx_builder.calldata().to_owned();

    let internal_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*multicall_contract.address())
        .with_input(internal_call_tx_calldata);

    let js_tracer_code = r#"
{
data: [],
step: function(log) {
    var op = log.op.toString();
    if (op === "SLOAD") {
    this.data.push(log.getPC() + ": SLOAD " + log.contract.getAddress() + ":" + log.stack.peek(0));
    this.data.push("    Result: " + log.stack.peek(0));
    } else if (op === "SSTORE") {
    this.data.push(log.getPC() + ": SSTORE " + log.contract.getAddress() + ":" + log.stack.peek(1) + " <- " + log.stack.peek(0));
    }
},
result: function() {
    return this.data;
},
fault: function(log) {}
}
"#;

    let result = api
        .debug_trace_call(
            WithOtherFields::new(internal_call_tx),
            Some(BlockId::latest()),
            GethDebugTracingCallOptions::default()
                .with_tracing_options(GethDebugTracingOptions::js_tracer(js_tracer_code)),
        )
        .await
        .unwrap();

    let expected = vec![
        "547: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:0",
        "    Result: 0",
        "1907: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:1",
        "    Result: 1",
        "772: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:1",
        "    Result: 1",
        "835: SSTORE 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:44498830125527143464827115118378702402016761369235290884359940707316142178310 <- 1",
        "919: SSTORE 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:0 <- 80084422859880547211683076133703299733277748156566366325829078699459944778998",
        "712: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:0",
        "    Result: 0",
        "765: SSTORE 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:546584486846459126461364135121053344201067465379 <- 0",
    ];

    let actual: Vec<String> = result
        .try_into_json_value()
        .ok()
        .and_then(|val| val.as_array().cloned())
        .map(|arr| arr.into_iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    assert_eq!(actual, expected);
}

#[cfg(feature = "js-tracer")]
#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_transaction_js_tracer() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = crate::utils::http_provider(&handle.http_endpoint());

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    api.anvil_add_balance(from, U256::MAX).await.unwrap();
    api.anvil_add_balance(wallets[1].address(), U256::MAX).await.unwrap();

    let multicall_contract = Multicall::deploy(&provider).await.unwrap();
    let simple_storage_contract =
        SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();

    let set_value = simple_storage_contract.setValue("bar".to_string());
    let set_value_calldata = set_value.calldata();

    let internal_call_tx_builder = multicall_contract.aggregate(vec![Multicall::Call {
        target: *simple_storage_contract.address(),
        callData: set_value_calldata.to_owned(),
    }]);

    let internal_call_tx_calldata = internal_call_tx_builder.calldata().to_owned();

    let internal_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*multicall_contract.address())
        .with_input(internal_call_tx_calldata)
        .with_gas_limit(1_000_000)
        .with_max_fee_per_gas(100_000_000_000)
        .with_max_priority_fee_per_gas(100_000_000_000);

    let receipt = provider
        .send_transaction(internal_call_tx.into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let js_tracer_code = r#"
{
data: [],
step: function(log) {
    var op = log.op.toString();
    var pc = log.getPC();
    var addr = log.contract.getAddress();

    if (op === "SLOAD") {
        this.data.push(pc + ": SLOAD " + addr + ":" + log.stack.peek(0));
        this.data.push("    Result: " + log.stack.peek(0));
    } else if (op === "SSTORE") {
        this.data.push(pc + ": SSTORE " + addr + ":" + log.stack.peek(1) + " <- " + log.stack.peek(0));
    } 
},
result: function() {
    return this.data;
},
fault: function(log) {}
}
"#;

    let expected = vec![
        "547: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:0",
        "    Result: 0",
        "1907: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:1",
        "    Result: 1",
        "772: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:1",
        "    Result: 1",
        "835: SSTORE 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:44498830125527143464827115118378702402016761369235290884359940707316142178310 <- 1",
        "919: SSTORE 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:0 <- 80084422859880547211683076133703299733277748156566366325829078699459944778998",
        "712: SLOAD 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:0",
        "    Result: 0",
        "765: SSTORE 231,241,114,94,119,52,206,40,143,131,103,225,187,20,62,144,187,63,5,18:546584486846459126461364135121053344201067465379 <- 0",
    ];
    let result = api
        .debug_trace_transaction(
            receipt.transaction_hash,
            GethDebugTracingOptions::js_tracer(js_tracer_code),
        )
        .await
        .unwrap();

    let actual: Vec<String> = result
        .try_into_json_value()
        .ok()
        .and_then(|val| val.as_array().cloned())
        .map(|arr| arr.into_iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    assert_eq!(actual, expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_tracer_debug_trace_call_pre_state_tracer() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let multicall_contract = Multicall::deploy(&provider).await.unwrap();
    let simple_storage_contract =
        SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();

    let set_value = simple_storage_contract.setValue("bar".to_string());
    let set_value_calldata = set_value.calldata();

    let internal_call_tx_builder = multicall_contract.aggregate(vec![Multicall::Call {
        target: *simple_storage_contract.address(),
        callData: set_value_calldata.to_owned(),
    }]);

    let internal_call_tx_calldata = internal_call_tx_builder.calldata().to_owned();

    let internal_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*multicall_contract.address())
        .with_input(internal_call_tx_calldata);

    let result = api
        .debug_trace_call(
            WithOtherFields::new(internal_call_tx),
            Some(BlockId::latest()),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::prestate_tracer(PreStateConfig::default()),
            ),
        )
        .await
        .unwrap();

    let expected = r#"
{
  "0x0000000000000000000000000000000000000000": {
    "balance": "0x12670f"
  },
  "0x5fbdb2315678afecb367f032d93f642f64180aa3": {
    "balance": "0x0",
    "nonce": 1
  },
  "0x70997970c51812dc3a010c7d01b50e0d17dc79c8": {
    "balance": "0x56bc75e2d63100000"
  },
  "0xe7f1725e7734ce288f8367e1bb143e90bb3f0512": {
    "balance": "0x0",
    "nonce": 1,
    "storage": {
      "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "0x0000000000000000000000000000000000000000000000000000000000000001": "0x696e69742076616c756500000000000000000000000000000000000000000014",
      "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6": "0x0000000000000000000000000000000000000000000000000000000000000000"
    }
  }
}
    "#;
    let expected: HashMap<Address, AccountState> = serde_json::from_str(expected).unwrap();

    match result {
        GethTrace::PreStateTracer(PreStateFrame::Default(pre_state_mode)) => {
            for (addr, acc) in pre_state_mode.0 {
                let expected_acc = expected.get(&addr).unwrap();
                assert_eq!(acc.balance, expected_acc.balance);
                assert_eq!(acc.nonce, expected_acc.nonce);
                let expected_storage = &expected_acc.storage;
                for (slot, value) in acc.storage {
                    assert_eq!(value, *expected_storage.get(&slot).unwrap())
                }
            }
        }
        _ => unreachable!(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_transaction_pre_state_tracer() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = crate::utils::http_provider(&handle.http_endpoint());

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let from = wallets[0].address();
    api.anvil_add_balance(from, U256::MAX).await.unwrap();
    api.anvil_add_balance(wallets[1].address(), U256::MAX).await.unwrap();

    let multicall_contract = Multicall::deploy(&provider).await.unwrap();
    let simple_storage_contract =
        SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();

    let set_value = simple_storage_contract.setValue("bar".to_string());
    let set_value_calldata = set_value.calldata();

    let internal_call_tx_builder = multicall_contract.aggregate(vec![Multicall::Call {
        target: *simple_storage_contract.address(),
        callData: set_value_calldata.to_owned(),
    }]);

    let internal_call_tx_calldata = internal_call_tx_builder.calldata().to_owned();

    let internal_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*multicall_contract.address())
        .with_input(internal_call_tx_calldata)
        .with_gas_limit(1_000_000)
        .with_max_fee_per_gas(100_000_000_000)
        .with_max_priority_fee_per_gas(100_000_000_000);

    let receipt = provider
        .send_transaction(internal_call_tx.into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let result = api
        .debug_trace_transaction(
            receipt.transaction_hash,
            GethDebugTracingOptions::prestate_tracer(PreStateConfig::default()),
        )
        .await
        .unwrap();

    let expected = r#"
{
  "0x0000000000000000000000000000000000000000": {
    "balance": "1206031000000000"
  },
  "0x5fbdb2315678afecb367f032d93f642f64180aa3": {
    "balance": "0x0",
    "nonce": 1
  },
  "0x70997970c51812dc3a010c7d01b50e0d17dc79c8": {
    "balance": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
  },
  "0xe7f1725e7734ce288f8367e1bb143e90bb3f0512": {
    "balance": "0x0",
    "nonce": 1,
    "storage": {
      "0x0000000000000000000000000000000000000000000000000000000000000000": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "0x0000000000000000000000000000000000000000000000000000000000000001": "0x696e69742076616c756500000000000000000000000000000000000000000014",
      "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6": "0x0000000000000000000000000000000000000000000000000000000000000000"
    }
  }
}
    "#;
    let expected: HashMap<Address, AccountState> = serde_json::from_str(expected).unwrap();

    match result {
        GethTrace::PreStateTracer(PreStateFrame::Default(pre_state_mode)) => {
            for (addr, acc) in pre_state_mode.0 {
                let expected_acc = expected.get(&addr).unwrap();
                assert_eq!(acc.balance, expected_acc.balance);
                assert_eq!(acc.nonce, expected_acc.nonce);
                let expected_storage = &expected_acc.storage;
                for (slot, value) in acc.storage {
                    assert_eq!(value, *expected_storage.get(&slot).unwrap())
                }
            }
        }
        _ => unreachable!(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_replay_block_transactions_local() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = U256::from(1000000u64);

    // Send first transaction
    let tx1 = TransactionRequest::default().to(to).value(amount).from(from);
    let tx1 = WithOtherFields::new(tx1);
    let pending_tx1 = provider.send_transaction(tx1).await.unwrap();

    // Send second transaction with different value
    let tx2 = TransactionRequest::default().to(to).value(amount).from(from);
    let tx2 = WithOtherFields::new(tx2);
    let pending_tx2 = provider.send_transaction(tx2).await.unwrap();

    api.mine_one().await.unwrap();
    let receipt1 = pending_tx1.get_receipt().await.unwrap();
    let receipt2 = pending_tx2.get_receipt().await.unwrap();

    let block_number = receipt2.block_number.unwrap();

    // Replay the block transactions with call trace type
    // Pass block number as hex string as per Ethereum RPC spec
    let results = api
        .trace_replay_block_transactions(
            block_number.into(),
            vec![TraceType::Trace, TraceType::VmTrace, TraceType::StateDiff].into_iter().collect(),
        )
        .await
        .unwrap()
        .unwrap();

    // Verify we have traces for both transactions
    assert_eq!(results.len(), 2, "Should have traces for 2 transactions");

    // Verify first transaction hash matches
    assert_eq!(results[0].transaction_hash, receipt1.transaction_hash);

    // Verify second transaction hash matches
    assert_eq!(results[1].transaction_hash, receipt2.transaction_hash);

    // Verify trace types are present and accurate
    for result in results {
        let full_trace = &result.full_trace;

        // Verify Trace (call trace) is present and accurate
        assert!(!full_trace.trace.is_empty(), "Trace should not be empty");
        let first_trace = &full_trace.trace[0];
        match &first_trace.action {
            Action::Call(call) => {
                assert_eq!(call.from, from, "Call from address should match");
                assert_eq!(call.to, to, "Call to address should match");
            }
            _ => panic!("Expected Call action, got {:?}", first_trace.action),
        }

        // Verify VmTrace is present
        assert!(full_trace.vm_trace.is_some(), "VmTrace should be present when requested");

        // Verify StateDiff is present
        assert!(full_trace.state_diff.is_some(), "StateDiff should be present when requested");
        // Verify balance change is correct in state diff
        let ChangedType::<U256> { from, to } =
            full_trace.state_diff.as_ref().unwrap().get(&to).unwrap().balance.as_changed().unwrap();
        assert_eq!(
            to.checked_sub(*from).unwrap(),
            amount,
            "Incorrect balance change in state diff"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_replay_transaction() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = U256::from(12345);

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let TraceResultsWithTransactionHash { full_trace: result, transaction_hash } = provider
        .client()
        .request(
            "trace_replayTransaction",
            (receipt.transaction_hash, vec![TraceType::Trace, TraceType::StateDiff]),
        )
        .await
        .unwrap();

    assert_eq!(transaction_hash, receipt.transaction_hash);
    assert!(!result.trace.is_empty());
    match &result.trace[0].action {
        Action::Call(call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
        }
        _ => panic!("Expected Call action, got {:?}", result.trace[0].action),
    }

    let ChangedType::<U256> { from, to } =
        result.state_diff.as_ref().unwrap().get(&to).unwrap().balance.as_changed().unwrap();
    assert_eq!(to.checked_sub(*from).unwrap(), amount);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_replay_transaction_fork() {
    let (_origin_api, origin_handle) = spawn(NodeConfig::test()).await;
    let origin = origin_handle.http_provider();
    let accounts = origin_handle.dev_wallets().collect::<Vec<_>>();
    let tx = TransactionRequest::default()
        .to(accounts[1].address())
        .value(U256::from(1000))
        .from(accounts[0].address());
    let receipt = origin
        .send_transaction(WithOtherFields::new(tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let hash = receipt.transaction_hash;

    let config = NodeConfig::test().with_eth_rpc_url(Some(origin_handle.http_endpoint()));
    let (_api, handle) = spawn(config).await;

    // The pre-fork transaction is replayed upstream and keeps its hash.
    let mut replays = Vec::new();
    for provider in [handle.http_provider(), origin] {
        replays.push(
            provider
                .client()
                .request::<_, TraceResultsWithTransactionHash>(
                    "trace_replayTransaction",
                    (hash, vec![TraceType::Trace]),
                )
                .await
                .unwrap(),
        );
    }
    assert_eq!(replays[0].transaction_hash, hash);
    assert_eq!(replays[0], replays[1]);

    // A hash unknown upstream as well is null.
    let unknown = handle
        .http_provider()
        .client()
        .request::<_, Option<TraceResultsWithTransactionHash>>(
            "trace_replayTransaction",
            (B256::ZERO, vec![TraceType::Trace]),
        )
        .await
        .unwrap();
    assert_eq!(unknown, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_replay_state_diff_account_lifecycle() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();
    let fresh = Address::repeat_byte(0x66);
    let nonce = provider.get_transaction_count(from).await.unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();

    let create = TransactionRequest::default()
        .from(from)
        .nonce(nonce)
        .with_deploy_code(Bytes::from_hex(INIT_RETURNING_2A).unwrap());
    let create = provider.send_transaction(WithOtherFields::new(create)).await.unwrap();
    let fund =
        TransactionRequest::default().from(from).to(fresh).nonce(nonce + 1).value(U256::from(7));
    let fund = provider.send_transaction(WithOtherFields::new(fund)).await.unwrap();
    let update =
        TransactionRequest::default().from(from).to(fresh).nonce(nonce + 2).value(U256::from(7));
    let update = provider.send_transaction(WithOtherFields::new(update)).await.unwrap();
    api.mine_one().await.unwrap();

    let create = create.get_receipt().await.unwrap();
    let fund = fund.get_receipt().await.unwrap();
    let update = update.get_receipt().await.unwrap();
    let created = create.contract_address.unwrap();
    let expected = [
        (create.transaction_hash, created, added_account("0x0", "0x1", "0x2a")),
        (fund.transaction_hash, fresh, added_account("0x7", "0x0", "0x")),
        (update.transaction_hash, fresh, changed_balance("0x7", "0xe")),
    ];

    let block = api
        .trace_replay_block_transactions(
            create.block_number.unwrap().into(),
            [TraceType::StateDiff].into_iter().collect(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.len(), expected.len());

    for (index, (hash, address, diff)) in expected.into_iter().enumerate() {
        let traces: TraceResults = provider
            .client()
            .request("trace_replayTransaction", (hash, vec![TraceType::StateDiff]))
            .await
            .unwrap();
        assert_eq!(state_diff_entry(&traces, address), Some(diff.clone()));
        assert_eq!(block[index].transaction_hash, hash);
        assert_eq!(state_diff_entry(&block[index].full_trace, address), Some(diff));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_replay_transaction_preserves_prefix_state() {
    let (api, handle) = spawn(NodeConfig::test().with_steps_tracing(true)).await;
    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();
    let contract = Address::random();
    // Return the old slot value and store calldata[0], reverting after the write if it is zero.
    api.anvil_set_code(
        contract,
        Bytes::from_hex("600054600052600035806000551560165760206000f35b60006000fd").unwrap(),
    )
    .await
    .unwrap();
    api.anvil_set_storage_at(contract, U256::ZERO, B256::with_last_byte(3)).await.unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();

    let values = [5u64, 0, 13, 8];
    let mut hashes = Vec::new();
    for (nonce, value) in values.into_iter().enumerate() {
        let tx = TransactionRequest::default()
            .from(from)
            .to(contract)
            .nonce(nonce as u64)
            .gas_limit(100_000)
            .input(Bytes::from(U256::from(value).to_be_bytes::<32>()).into());
        hashes.push(api.send_transaction(WithOtherFields::new(tx)).await.unwrap());
    }
    api.mine_one().await.unwrap();
    let block_number = provider.get_block_number().await.unwrap();
    let old_values = [3u64, 5, 5, 13];

    for trace_types in [
        vec![TraceType::Trace, TraceType::VmTrace, TraceType::StateDiff],
        vec![TraceType::Trace],
        vec![TraceType::VmTrace],
        vec![TraceType::StateDiff],
        vec![],
    ] {
        let block_results = api
            .trace_replay_block_transactions(
                block_number.into(),
                trace_types.iter().copied().collect(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(block_results.len(), hashes.len());
        for (index, hash) in hashes.iter().copied().enumerate() {
            let result: TraceResults = provider
                .client()
                .request("trace_replayTransaction", (hash, &trace_types))
                .await
                .unwrap();
            assert_eq!(block_results[index].transaction_hash, hash);
            assert_eq!(result, block_results[index].full_trace);
            if values[index] == 0 {
                assert!(result.output.is_empty());
                if trace_types.contains(&TraceType::Trace) {
                    assert!(result.trace[0].error.is_some());
                }
            } else {
                assert_eq!(
                    result.output.as_ref(),
                    U256::from(old_values[index]).to_be_bytes::<32>()
                );
                if let Some(state_diff) = &result.state_diff {
                    let change = state_diff[&contract].storage[&B256::ZERO].as_changed().unwrap();
                    assert_eq!(change.from, B256::from(U256::from(old_values[index])));
                    assert_eq!(change.to, B256::from(U256::from(values[index])));
                }
            }
        }
    }
    let block = api.backend.get_block(BlockId::number(block_number)).unwrap();
    let mut rlp_block = Vec::new();
    block.encode(&mut rlp_block);
    for options in [
        serde_json::json!({"tracer": "callTracer"}),
        serde_json::json!({"tracer": "callTracer", "tracerConfig": {"withLog": true}}),
        serde_json::json!({"tracer": "callTracer", "tracerConfig": {"onlyTopCall": true}}),
        serde_json::json!({"tracer": "callTracer", "tracerConfig": {"onlyTopLevelCall": true}}),
        serde_json::json!({"tracer": "callTracer", "tracerConfig": {"withLog": "invalid"}}),
        serde_json::json!({"tracer": "noopTracer"}),
        serde_json::json!({}),
        serde_json::json!({"enableMemory": true, "enableReturnData": true}),
    ] {
        let options = serde_json::from_value::<GethDebugTracingOptions>(options).unwrap();
        let mut expected = Vec::new();
        for hash in &hashes {
            expected.push(
                match api.backend.debug_trace_transaction(*hash, options.clone()).await {
                    Ok(result) => TraceResult::Success { result, tx_hash: Some(*hash) },
                    Err(error) => {
                        TraceResult::Error { error: error.to_string(), tx_hash: Some(*hash) }
                    }
                },
            );
        }
        assert_eq!(
            api.backend
                .debug_trace_block_by_number(block_number.into(), options.clone())
                .await
                .unwrap(),
            expected,
        );
        assert_eq!(
            api.backend
                .debug_trace_block_by_hash(block.header.hash_slow(), options.clone())
                .await
                .unwrap(),
            expected,
        );
        assert_eq!(
            api.backend.debug_trace_block(rlp_block.clone().into(), options).await.unwrap(),
            expected,
        );
    }
    // Replays must not mutate the live chain.
    assert_eq!(provider.get_storage_at(contract, U256::ZERO).await.unwrap(), U256::from(8));
    assert_eq!(provider.get_transaction_count(from).await.unwrap(), hashes.len() as u64);
    assert_eq!(provider.get_block_number().await.unwrap(), block_number);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_block_without_history() {
    let (api, handle) = spawn(NodeConfig::test().set_pruned_history(Some(None))).await;
    let from = handle.dev_wallets().next().unwrap().address();
    let invalid = serde_json::from_value::<GethDebugTracingOptions>(serde_json::json!({
        "tracer": "callTracer", "tracerConfig": {"withLog": "invalid"},
    }))
    .unwrap();
    assert!(
        api.backend
            .debug_trace_block_by_number(0.into(), invalid.clone())
            .await
            .unwrap()
            .is_empty()
    );

    api.anvil_set_auto_mine(false).await.unwrap();
    let mut hashes = Vec::new();
    for nonce in 0..2 {
        hashes.push(
            api.send_transaction(WithOtherFields::new(
                TransactionRequest::default().from(from).to(from).nonce(nonce).gas_limit(21_000),
            ))
            .await
            .unwrap(),
        );
    }
    api.mine_one().await.unwrap();
    let receipt = handle.http_provider().get_transaction_receipt(hashes[0]).await.unwrap().unwrap();
    api.mine_one().await.unwrap();
    for options in [
        GethDebugTracingOptions::default()
            .with_tracer(GethDebugBuiltInTracerType::CallTracer.into()),
        invalid,
        GethDebugTracingOptions::default(),
        GethDebugTracingOptions::default()
            .with_tracer(GethDebugBuiltInTracerType::NoopTracer.into()),
    ] {
        let mut expected = Vec::new();
        for hash in &hashes {
            let trace = match api.backend.debug_trace_transaction(*hash, options.clone()).await {
                Ok(result) => TraceResult::Success { result, tx_hash: Some(*hash) },
                Err(error) => TraceResult::Error { error: error.to_string(), tx_hash: Some(*hash) },
            };
            assert_eq!(
                trace.is_error(),
                matches!(
                    options.tracer,
                    Some(GethDebugTracerType::BuiltInTracer(
                        GethDebugBuiltInTracerType::CallTracer
                    ))
                ),
            );
            expected.push(trace);
        }
        assert_eq!(
            api.backend
                .debug_trace_block_by_hash(receipt.block_hash.unwrap(), options.clone())
                .await
                .unwrap(),
            expected.clone(),
        );
        assert_eq!(
            api.backend
                .debug_trace_block_by_number(receipt.block_number.unwrap().into(), options)
                .await
                .unwrap(),
            expected,
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_block_by_number() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = U256::from(1000);

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let block_number = receipt.block_number.unwrap();

    let traces = api
        .backend
        .debug_trace_block_by_number(
            BlockNumberOrTag::Number(block_number),
            GethDebugTracingOptions::default()
                .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer)),
        )
        .await
        .unwrap();

    assert_eq!(traces.len(), 1);

    match &traces[0] {
        alloy_rpc_types::trace::geth::TraceResult::Success { result, .. } => match result {
            GethTrace::CallTracer(call_frame) => {
                assert_eq!(call_frame.from, from);
                assert_eq!(call_frame.to.unwrap(), to);
                assert_eq!(call_frame.value, Some(amount));
            }
            _ => unreachable!("expected CallTracer"),
        },
        alloy_rpc_types::trace::geth::TraceResult::Error { error, .. } => {
            panic!("trace failed: {error}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_block_by_hash() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = U256::from(2000);

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let block_hash = receipt.block_hash.unwrap();

    let traces = api
        .backend
        .debug_trace_block_by_hash(
            block_hash,
            GethDebugTracingOptions::default()
                .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer)),
        )
        .await
        .unwrap();

    assert_eq!(traces.len(), 1);

    match &traces[0] {
        alloy_rpc_types::trace::geth::TraceResult::Success { result, .. } => match result {
            GethTrace::CallTracer(call_frame) => {
                assert_eq!(call_frame.from, from);
                assert_eq!(call_frame.to.unwrap(), to);
                assert_eq!(call_frame.value, Some(amount));
            }
            _ => unreachable!("expected CallTracer"),
        },
        alloy_rpc_types::trace::geth::TraceResult::Error { error, .. } => {
            panic!("trace failed: {error}");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_debug_trace_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = U256::from(3000);

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let block_hash = receipt.block_hash.unwrap();
    let block = api.backend.get_block(BlockId::hash(block_hash)).unwrap();

    let mut rlp_block = Vec::new();
    block.encode(&mut rlp_block);

    let traces = provider
        .debug_trace_block(
            &rlp_block,
            GethDebugTracingOptions::default()
                .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer)),
        )
        .await
        .unwrap();

    assert_eq!(traces.len(), 1);

    match &traces[0] {
        alloy_rpc_types::trace::geth::TraceResult::Success { result, .. } => match result {
            GethTrace::CallTracer(call_frame) => {
                assert_eq!(call_frame.from, from);
                assert_eq!(call_frame.to.unwrap(), to);
                assert_eq!(call_frame.value, Some(amount));
            }
            _ => unreachable!("expected CallTracer"),
        },
        alloy_rpc_types::trace::geth::TraceResult::Error { error, .. } => {
            panic!("trace failed: {error}");
        }
    }
}
