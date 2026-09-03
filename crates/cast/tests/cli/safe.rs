use alloy_consensus::Transaction;
use alloy_dyn_abi::TypedData;
use alloy_eips::Typed2718;
use alloy_network::ReceiptResponse;
use alloy_primitives::{Address, B256, Bytes, Signature, U256, address, b256, hex, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::BlockNumberOrTag;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, SolEvent};
use anvil::NodeConfig;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{RawQuery, State},
    http::{Method, Request, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use foundry_cli::json::JsonEnvelope;
use foundry_test_utils::util::OutputExt;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use test_safe_contract::TestSafe;
use tokio::task::JoinHandle;

const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ANVIL_KEY_2: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const ANVIL_KEY_3: &str = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";
const ANVIL_OWNER: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
const ANVIL_OWNER_2: Address = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");
const ANVIL_OWNER_3: Address = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC");
const SAFE_L2_V1_4_1: Address = address!("29fcB43b46531BcA003ddC8FCB67FFE91900C762");
const SAFE_PROXY_FACTORY_V1_4_1: Address = address!("4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67");
const SIMULATE_TX_ACCESSOR_V1_4_1: Address = address!("3d4BA2E0884aa488718476ca2FB8Efc291A46199");
const SAFE_L2_V1_4_1_RUNTIME_LEN: usize = 24_421;
const SAFE_PROXY_FACTORY_V1_4_1_RUNTIME_LEN: usize = 3_054;
const SIMULATE_TX_ACCESSOR_V1_4_1_RUNTIME_LEN: usize = 850;
const SAFE_L2_V1_4_1_RUNTIME_HASH: B256 =
    b256!("b1f926978a0f44a2c0ec8fe822418ae969bd8c3f18d61e5103100339894f81ff");
const SAFE_PROXY_FACTORY_V1_4_1_RUNTIME_HASH: B256 =
    b256!("50c3cdc4074750a7a974204a716c999edd37482f907608d960b2b025ee0b3317");
const SIMULATE_TX_ACCESSOR_V1_4_1_RUNTIME_HASH: B256 =
    b256!("91f82615581fc73b190b83d72e883608b25e392f72322035df1b13d51766cf8d");
const SAFE_SERVICE_API_KEY: &str = "cast-safe-test-key";
const SIMULATION_SAFE_CODE: &str = "0x63d8d11f785f3560e01c146051577333333333333333333333333333333333333333333214602b575f80fd5b60015f5260a0602052602a60405260016060526060608052602060a0523260c05260e05ffd5b5f805260205ff3";

#[allow(clippy::too_many_arguments)]
mod test_safe_contract {
    alloy_sol_types::sol! {
        #[sol(rpc)]
        interface TestSafe {
            function nonce() external view returns (uint256);

            function getOwners() external view returns (address[] memory);

            function getThreshold() external view returns (uint256);

            function getTransactionHash(
                address to,
                uint256 value,
                bytes calldata data,
                uint8 operation,
                uint256 safeTxGas,
                uint256 baseGas,
                uint256 gasPrice,
                address gasToken,
                address refundReceiver,
                uint256 nonce
            ) external view returns (bytes32);

            function execTransaction(
                address to,
                uint256 value,
                bytes calldata data,
                uint8 operation,
                uint256 safeTxGas,
                uint256 baseGas,
                uint256 gasPrice,
                address gasToken,
                address payable refundReceiver,
                bytes calldata signatures
            ) external payable returns (bool success);

            event ExecutionSuccess(bytes32 indexed txHash, uint256 payment);
        }
    }
}

fn safe_transaction(safe: Address, operation: u8) -> Value {
    json!({
        "safe": safe,
        "to": Address::ZERO,
        "value": "0",
        "data": "0x",
        "operation": operation,
        "safeTxGas": "0",
        "baseGas": "0",
        "gasPrice": "0",
        "gasToken": Address::ZERO,
        "refundReceiver": Address::ZERO,
        "nonce": "0",
        "safeTxHash": B256::ZERO,
    })
}

async fn spawn_safe_service(response: Value) -> TestServerHandle {
    let router = Router::new().fallback(move || {
        let response = response.clone();
        async move { Json(response) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    TestServerHandle { endpoint, task }
}

fn json_envelope(data: Value) -> String {
    serde_json::to_string(&JsonEnvelope::success(data)).unwrap()
}

fn fixture_runtime(bytes: &'static [u8], expected_len: usize, expected_hash: B256) -> Bytes {
    assert_eq!(bytes.len(), expected_len, "Safe runtime fixture length changed");
    assert_eq!(keccak256(bytes), expected_hash, "Safe runtime fixture hash changed");
    Bytes::from_static(bytes)
}

#[derive(Debug, Clone)]
struct StrictConfirmation {
    owner: Address,
    signature: String,
}

#[derive(Debug, Clone)]
struct StrictTransaction {
    safe: Address,
    to: Address,
    value: String,
    data: String,
    operation: u8,
    safe_tx_gas: String,
    base_gas: String,
    gas_price: String,
    gas_token: Address,
    refund_receiver: Address,
    nonce: u64,
    hash: B256,
    proposal_sender: Address,
    proposal_signature: String,
    confirmations: Vec<StrictConfirmation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExpectedTransaction {
    safe: Address,
    target: Address,
    nonce: u64,
    hash: B256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RequestEvent {
    AddDelegate { safe: Address, delegate: Address, delegator: Address },
    ListDelegates { safe: Address },
    NonceLookup { safe: Address },
    Propose { safe: Address, sender: Address, hash: B256 },
    GetTransaction { version: u8, hash: B256 },
    Confirm { hash: B256, owner: Address },
    RemoveDelegate { safe: Address, delegate: Address, delegator: Address },
}

#[derive(Debug)]
struct StrictServiceState {
    chain_id: u64,
    api_key: String,
    owners: Vec<Address>,
    expected: ExpectedTransaction,
    delegates: Vec<(Address, Address, String)>,
    transaction: Option<StrictTransaction>,
    requests: Vec<RequestEvent>,
}

struct TestServerHandle {
    endpoint: String,
    task: JoinHandle<()>,
}

impl TestServerHandle {
    fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for TestServerHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A small, stateful Transaction Service double used by the lifecycle test.
///
/// It deliberately accepts only the routes used by Cast and checks the wire-level
/// representation, so a test cannot pass by talking to a permissive catch-all.
struct SafeServiceHandle {
    state: Arc<Mutex<StrictServiceState>>,
    server: TestServerHandle,
}

impl SafeServiceHandle {
    fn endpoint(&self) -> &str {
        self.server.endpoint()
    }

    fn assert_delegate_proposal(&self, delegate: Address, hash: B256) {
        let state = self.state.lock().unwrap();
        assert!(
            state.delegates.iter().any(|(current, _, _)| *current == delegate),
            "delegate proposal was accepted without a registered delegate"
        );
        let transaction = state.transaction.as_ref().expect("transaction was not proposed");
        assert_eq!(transaction.hash, hash);
        assert_eq!(transaction.proposal_sender, delegate);
        assert!(!transaction.proposal_signature.is_empty());
        ensure_safe_signature(&transaction.proposal_signature, hash, delegate)
            .expect("delegate proposal signature did not recover the delegate");
        assert!(
            transaction.confirmations.is_empty(),
            "delegate proposal must not count as an owner confirmation"
        );
    }

    fn assert_lifecycle_complete(&self, expected_requests: &[RequestEvent], delegate: Address) {
        let state = self.state.lock().unwrap();
        assert!(state.delegates.is_empty(), "delegate was not removed: {:?}", state.delegates);
        let transaction = state.transaction.as_ref().expect("transaction was not proposed");
        let expected = state.expected;
        assert_eq!(transaction.safe, expected.safe);
        assert_eq!(transaction.to, expected.target);
        assert_eq!(transaction.nonce, expected.nonce);
        assert_eq!(transaction.hash, expected.hash);
        assert_eq!(transaction.proposal_sender, delegate);
        assert_eq!(transaction.confirmations.len(), state.owners.len());
        assert!(
            !transaction.confirmations.iter().any(|confirmation| confirmation.owner == delegate)
        );
        let mut actual_owners = transaction
            .confirmations
            .iter()
            .map(|confirmation| confirmation.owner)
            .collect::<Vec<_>>();
        actual_owners.sort_unstable();
        let mut expected_owners = state.owners.clone();
        expected_owners.sort_unstable();
        assert_eq!(actual_owners, expected_owners);
        assert_eq!(state.requests, expected_requests);
    }
}

async fn spawn_strict_safe_service(
    owners: Vec<Address>,
    chain_id: u64,
    expected: ExpectedTransaction,
) -> SafeServiceHandle {
    let state = Arc::new(Mutex::new(StrictServiceState {
        chain_id,
        api_key: SAFE_SERVICE_API_KEY.to_string(),
        owners,
        expected,
        delegates: Vec::new(),
        transaction: None,
        requests: Vec::new(),
    }));
    let router = Router::new().fallback(strict_service_handler).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let server = TestServerHandle { endpoint, task };
    SafeServiceHandle { state, server }
}

async fn strict_service_handler(
    State(state): State<Arc<Mutex<StrictServiceState>>>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = match to_bytes(request.into_body(), 1024 * 1024).await {
        Ok(body) => body,
        Err(error) => return service_error(format!("failed to read request body: {error}")),
    };
    let body = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(body) => body,
            Err(error) => return service_error(format!("invalid JSON request body: {error}")),
        }
    };

    let mut state = state.lock().unwrap();
    if authorization.as_deref() != Some(&format!("Bearer {}", state.api_key)) {
        return service_error_with_status(StatusCode::UNAUTHORIZED, "invalid API authorization");
    }
    match strict_service_request(&mut state, method, &uri, body) {
        Ok(ServiceResponse::Json(value)) => Json(value).into_response(),
        Ok(ServiceResponse::Empty) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => service_error(error),
    }
}

enum ServiceResponse {
    Json(Value),
    Empty,
}

fn strict_service_request(
    state: &mut StrictServiceState,
    method: Method,
    uri: &Uri,
    body: Value,
) -> Result<ServiceResponse, String> {
    let path = uri.path();
    let method_name = method.as_str();
    if path == "/api/v2/delegates/" {
        match method {
            Method::GET => {
                handle_list_delegates(state, uri)?;
                let safe = state.expected.safe;
                state.requests.push(RequestEvent::ListDelegates { safe });
                return Ok(ServiceResponse::Json(json!({
                    "count": state.delegates.len(),
                    "next": null,
                    "previous": null,
                    "results": state.delegates.iter().map(|(delegate, delegator, label)| json!({
                        "safe": checksum(safe),
                        "delegate": checksum(*delegate),
                        "delegator": checksum(*delegator),
                        "label": label,
                    })).collect::<Vec<_>>(),
                })));
            }
            Method::POST => {
                ensure_no_query(uri)?;
                let (delegate, delegator) = handle_add_delegate(state, body)?;
                let safe = state.expected.safe;
                state.requests.push(RequestEvent::AddDelegate { safe, delegate, delegator });
                return Ok(ServiceResponse::Empty);
            }
            _ => return Err(format!("unexpected method {method_name} for {path}")),
        }
    }
    if path == "/api/v2/safes/" {
        return Err("proposal path is missing its Safe address".to_string());
    }
    if let Some(safe_path) = path.strip_prefix("/api/v2/safes/")
        && let Some(suffix) = safe_path.strip_suffix("/multisig-transactions/")
    {
        ensure_method(&method, Method::POST, path)?;
        ensure_no_query(uri)?;
        let (sender, hash) = handle_proposal(state, suffix, body)?;
        let safe = state.expected.safe;
        state.requests.push(RequestEvent::Propose { safe, sender, hash });
        return Ok(ServiceResponse::Empty);
    }
    if let Some(safe_path) = path.strip_prefix("/api/v1/safes/")
        && let Some(suffix) = safe_path.strip_suffix("/multisig-transactions/")
    {
        ensure_method(&method, Method::GET, path)?;
        handle_nonce_lookup(state, suffix, uri)?;
        let safe = state.expected.safe;
        state.requests.push(RequestEvent::NonceLookup { safe });
        return Ok(ServiceResponse::Json(json!({
            "count": 0,
            "next": null,
            "previous": null,
            "results": [],
        })));
    }
    if let Some(tx_path) = path.strip_prefix("/api/v1/multisig-transactions/") {
        if let Some(hash) = tx_path.strip_suffix("/confirmations/") {
            ensure_method(&method, Method::POST, path)?;
            ensure_no_query(uri)?;
            let (hash, owner) = handle_confirmation(state, hash, body)?;
            state.requests.push(RequestEvent::Confirm { hash, owner });
            return Ok(ServiceResponse::Empty);
        }
        if let Some(hash) = tx_path.strip_suffix('/') {
            ensure_method(&method, Method::GET, path)?;
            ensure_no_query(uri)?;
            let transaction_ref = handle_transaction_get(state, hash)?;
            let transaction_hash = transaction_ref.hash;
            let transaction = transaction_json(transaction_ref);
            state
                .requests
                .push(RequestEvent::GetTransaction { version: 1, hash: transaction_hash });
            return Ok(ServiceResponse::Json(transaction));
        }
    }
    if let Some(tx_path) = path.strip_prefix("/api/v2/multisig-transactions/")
        && let Some(hash) = tx_path.strip_suffix('/')
    {
        ensure_method(&method, Method::GET, path)?;
        ensure_no_query(uri)?;
        let transaction_ref = handle_transaction_get(state, hash)?;
        let transaction_hash = transaction_ref.hash;
        let transaction = transaction_json(transaction_ref);
        state.requests.push(RequestEvent::GetTransaction { version: 2, hash: transaction_hash });
        return Ok(ServiceResponse::Json(transaction));
    }
    if let Some(delegate_path) = path.strip_prefix("/api/v2/delegates/")
        && let Some(delegate) = delegate_path.strip_suffix('/')
    {
        if method != Method::DELETE {
            return Err(format!("unexpected method {method_name} for {path}"));
        }
        ensure_no_query(uri)?;
        let (safe, delegate, delegator) = handle_remove_delegate(state, delegate, body)?;
        state.requests.push(RequestEvent::RemoveDelegate { safe, delegate, delegator });
        return Ok(ServiceResponse::Empty);
    }
    Err(format!("unexpected Transaction Service route: {method_name} {uri}"))
}

fn service_error(error: impl Into<String>) -> Response {
    service_error_with_status(StatusCode::BAD_REQUEST, error)
}

fn service_error_with_status(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(json!({ "detail": error.into() }))).into_response()
}

fn ensure_method(method: &Method, expected: Method, path: &str) -> Result<(), String> {
    if *method != expected {
        return Err(format!(
            "unexpected method {} for {path}; expected {}",
            method.as_str(),
            expected.as_str()
        ));
    }
    Ok(())
}

fn ensure_no_query(uri: &Uri) -> Result<(), String> {
    if let Some(query) = uri.query() {
        return Err(format!("unexpected query `{query}` for {}", uri.path()));
    }
    Ok(())
}

fn checksum(address: Address) -> String {
    address.to_checksum(None)
}

fn handle_list_delegates(state: &StrictServiceState, uri: &Uri) -> Result<(), String> {
    let safe = state.expected.safe;
    let expected = format!("safe={}", checksum(safe));
    if uri.query() != Some(expected.as_str()) {
        return Err(format!("delegate list query must be `{expected}`, got {:?}", uri.query()));
    }
    Ok(())
}

fn handle_add_delegate(
    state: &mut StrictServiceState,
    body: Value,
) -> Result<(Address, Address), String> {
    let safe = state.expected.safe;
    ensure_address_field(&body, "safe", safe)?;
    let delegate = parse_checksum_field(&body, "delegate")?;
    let delegator = parse_checksum_field(&body, "delegator")?;
    if !state.owners.contains(&delegator) {
        return Err(format!("delegator {delegator} is not a Safe owner"));
    }
    let label =
        body.get("label").and_then(Value::as_str).ok_or("delegate label is missing")?.to_owned();
    if label.trim().is_empty() {
        return Err("delegate label cannot be empty".to_string());
    }
    let signature =
        body.get("signature").and_then(Value::as_str).ok_or("delegate signature is missing")?;
    ensure_delegate_signature(signature, delegate, delegator, state.chain_id)?;
    if state.delegates.iter().any(|(current, _, _)| *current == delegate) {
        return Err(format!("delegate {delegate} was already registered"));
    }
    state.delegates.push((delegate, delegator, label));
    Ok((delegate, delegator))
}

fn handle_remove_delegate(
    state: &mut StrictServiceState,
    path: &str,
    body: Value,
) -> Result<(Address, Address, Address), String> {
    let safe = state.expected.safe;
    let delegate = parse_checksum_path(path, "delegate")?;
    ensure_address_field(&body, "safe", safe)?;
    let delegator = parse_checksum_field(&body, "delegator")?;
    if !state.owners.contains(&delegator) {
        return Err(format!("delegator {delegator} is not a Safe owner"));
    }
    let signature =
        body.get("signature").and_then(Value::as_str).ok_or("delegate signature is missing")?;
    ensure_delegate_signature(signature, delegate, delegator, state.chain_id)?;
    let before = state.delegates.len();
    state.delegates.retain(|(current, _, _)| *current != delegate);
    if state.delegates.len() == before {
        return Err(format!("delegate {delegate} was not registered"));
    }
    Ok((safe, delegate, delegator))
}

fn handle_nonce_lookup(state: &StrictServiceState, path: &str, uri: &Uri) -> Result<(), String> {
    let safe = state.expected.safe;
    ensure_checksum_path(path, safe, "Safe")?;
    let expected = "executed=false&ordering=-nonce&limit=1";
    if uri.query() != Some(expected) {
        return Err(format!("nonce lookup query must be `{expected}`, got {:?}", uri.query()));
    }
    Ok(())
}

fn handle_proposal(
    state: &mut StrictServiceState,
    path: &str,
    body: Value,
) -> Result<(Address, B256), String> {
    let expected = state.expected;
    let safe = expected.safe;
    ensure_checksum_path(path, safe, "Safe")?;
    if state.transaction.is_some() {
        return Err("only one Safe transaction is supported by this test service".to_string());
    }
    let target = parse_checksum_field(&body, "to")?;
    if target != expected.target {
        return Err(format!("proposal target {target} does not match {}", expected.target));
    }
    let operation =
        body.get("operation").and_then(Value::as_u64).ok_or("proposal operation is missing")?;
    if operation > 1 {
        return Err(format!("invalid proposal operation {operation}"));
    }
    let value = decimal_field(&body, "value")?;
    let safe_tx_gas = decimal_field(&body, "safeTxGas")?;
    let base_gas = decimal_field(&body, "baseGas")?;
    let gas_price = decimal_field(&body, "gasPrice")?;
    let nonce = decimal_field(&body, "nonce")?.parse::<u64>().map_err(|_| "invalid nonce")?;
    if nonce != expected.nonce {
        return Err(format!("proposal nonce {nonce} does not match {}", expected.nonce));
    }
    let data =
        body.get("data").and_then(Value::as_str).ok_or("proposal data is missing")?.to_owned();
    let gas_token = parse_checksum_field(&body, "gasToken")?;
    let refund_receiver = parse_checksum_field(&body, "refundReceiver")?;
    let hash = body
        .get("contractTransactionHash")
        .and_then(Value::as_str)
        .ok_or("proposal transaction hash is missing")?
        .parse::<B256>()
        .map_err(|error| format!("invalid proposal transaction hash: {error}"))?;
    if hash != expected.hash {
        return Err(format!("proposal hash {hash} does not match {}", expected.hash));
    }
    let sender = parse_checksum_field(&body, "sender")?;
    let is_owner = state.owners.contains(&sender);
    let is_delegate = state.delegates.iter().any(|(delegate, _, _)| *delegate == sender);
    if !is_owner && !is_delegate {
        return Err(format!("proposal sender {sender} is not an owner or registered delegate"));
    }
    let signature = body
        .get("signature")
        .and_then(Value::as_str)
        .ok_or("proposal signature is missing")?
        .to_owned();
    ensure_safe_signature(&signature, hash, sender)?;
    let confirmations = if is_owner {
        vec![StrictConfirmation { owner: sender, signature: signature.clone() }]
    } else {
        Vec::new()
    };
    state.transaction = Some(StrictTransaction {
        safe,
        to: target,
        value,
        data,
        operation: operation as u8,
        safe_tx_gas,
        base_gas,
        gas_price,
        gas_token,
        refund_receiver,
        nonce,
        hash,
        proposal_sender: sender,
        proposal_signature: signature,
        confirmations,
    });
    Ok((sender, hash))
}

fn handle_transaction_get<'a>(
    state: &'a StrictServiceState,
    path: &str,
) -> Result<&'a StrictTransaction, String> {
    let hash =
        path.parse::<B256>().map_err(|error| format!("invalid transaction hash path: {error}"))?;
    let transaction = state.transaction.as_ref().ok_or("transaction was not proposed")?;
    if transaction.hash != hash {
        return Err(format!("requested transaction {hash}, expected {}", transaction.hash));
    }
    Ok(transaction)
}

fn handle_confirmation(
    state: &mut StrictServiceState,
    path: &str,
    body: Value,
) -> Result<(B256, Address), String> {
    let hash = handle_transaction_get(state, path)?.hash;
    let signature = body
        .get("signature")
        .and_then(Value::as_str)
        .ok_or("confirmation signature is missing")?
        .to_owned();
    let owner = recover_safe_signature(&signature, hash)?;
    if !state.owners.contains(&owner) {
        return Err(format!("confirmation signer {owner} is not a Safe owner"));
    }
    let transaction = state.transaction.as_mut().expect("transaction was checked above");
    if transaction.confirmations.iter().any(|confirmation| confirmation.owner == owner) {
        return Err(format!("duplicate confirmation from {owner}"));
    }
    transaction.confirmations.push(StrictConfirmation { owner, signature });
    Ok((hash, owner))
}

fn transaction_json(transaction: &StrictTransaction) -> Value {
    json!({
        "safe": checksum(transaction.safe),
        "to": checksum(transaction.to),
        "value": transaction.value,
        "data": transaction.data,
        "operation": transaction.operation,
        "safeTxGas": transaction.safe_tx_gas,
        "baseGas": transaction.base_gas,
        "gasPrice": transaction.gas_price,
        "gasToken": checksum(transaction.gas_token),
        "refundReceiver": checksum(transaction.refund_receiver),
        "nonce": transaction.nonce.to_string(),
        "safeTxHash": transaction.hash,
        "confirmations": transaction.confirmations.iter().map(|confirmation| json!({
            "owner": checksum(confirmation.owner),
            "signature": confirmation.signature,
        })).collect::<Vec<_>>(),
        "isExecuted": false,
        "transactionHash": Value::Null,
    })
}

fn ensure_checksum_path(path: &str, expected: Address, name: &str) -> Result<(), String> {
    if path != checksum(expected) {
        return Err(format!("{name} path must use checksum {}, got {path}", checksum(expected)));
    }
    Ok(())
}

fn parse_checksum_path(path: &str, name: &str) -> Result<Address, String> {
    let address =
        path.parse::<Address>().map_err(|error| format!("invalid {name} path: {error}"))?;
    if path != checksum(address) {
        return Err(format!("{name} path is not checksummed: {path}"));
    }
    Ok(address)
}

fn parse_checksum_field(body: &Value, field: &str) -> Result<Address, String> {
    let value =
        body.get(field).and_then(Value::as_str).ok_or_else(|| format!("{field} is missing"))?;
    let address = value.parse::<Address>().map_err(|error| format!("invalid {field}: {error}"))?;
    if value != checksum(address) {
        return Err(format!("{field} is not checksummed: {value}"));
    }
    Ok(address)
}

fn ensure_address_field(body: &Value, field: &str, expected: Address) -> Result<(), String> {
    let actual = parse_checksum_field(body, field)?;
    if actual != expected {
        return Err(format!("{field} {actual} does not match {}", checksum(expected)));
    }
    Ok(())
}

fn decimal_field(body: &Value, field: &str) -> Result<String, String> {
    let value = body
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} must be a decimal string"))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{field} must be a decimal string"));
    }
    value.parse::<U256>().map_err(|error| format!("invalid {field}: {error}"))?;
    Ok(value.to_owned())
}

fn decode_signature(value: &str) -> Result<Vec<u8>, String> {
    let bytes =
        value.parse::<Bytes>().map_err(|error| format!("invalid signature encoding: {error}"))?;
    if bytes.len() != 65 {
        return Err(format!("expected 65-byte signature, got {}", bytes.len()));
    }
    Ok(bytes.to_vec())
}

fn recover_safe_signature(value: &str, hash: B256) -> Result<Address, String> {
    let mut bytes = decode_signature(value)?;
    if !matches!(bytes[64], 31 | 32) {
        return Err(format!("Safe signature has invalid v = {}", bytes[64]));
    }
    bytes[64] -= 4;
    let signature =
        Signature::from_raw(&bytes).map_err(|error| format!("invalid Safe signature: {error}"))?;
    signature
        .recover_address_from_msg(hash.as_slice())
        .map_err(|error| format!("failed to recover Safe signature: {error}"))
}

fn ensure_safe_signature(value: &str, hash: B256, expected: Address) -> Result<(), String> {
    let recovered = recover_safe_signature(value, hash)?;
    if recovered != expected {
        return Err(format!("signature recovered {recovered}, expected {}", checksum(expected)));
    }
    Ok(())
}

fn delegate_typed_hash(delegate: Address, chain_id: u64, totp: u64) -> Result<B256, String> {
    let typed_data: TypedData = serde_json::from_value(json!({
        "types": {
            "EIP712Domain": [
                { "name": "name", "type": "string" },
                { "name": "version", "type": "string" },
                { "name": "chainId", "type": "uint256" }
            ],
            "Delegate": [
                { "name": "delegateAddress", "type": "address" },
                { "name": "totp", "type": "uint256" }
            ]
        },
        "primaryType": "Delegate",
        "domain": {
            "name": "Safe Transaction Service",
            "version": "1.0",
            "chainId": chain_id
        },
        "message": {
            "delegateAddress": checksum(delegate),
            "totp": totp
        }
    }))
    .map_err(|error| format!("failed to build delegate typed data: {error}"))?;
    typed_data
        .eip712_signing_hash()
        .map_err(|error| format!("failed to hash delegate typed data: {error}"))
}

fn ensure_delegate_signature(
    value: &str,
    delegate: Address,
    expected: Address,
    chain_id: u64,
) -> Result<(), String> {
    let bytes = decode_signature(value)?;
    if !matches!(bytes[64], 27 | 28) {
        return Err(format!("delegate signature has invalid v = {}", bytes[64]));
    }
    let signature = Signature::from_raw(&bytes)
        .map_err(|error| format!("invalid delegate signature: {error}"))?;
    let current = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_secs()
        / (60 * 60);
    for totp in [current.saturating_sub(1), current, current.saturating_add(1)] {
        let hash = delegate_typed_hash(delegate, chain_id, totp)?;
        if signature.recover_address_from_prehash(&hash).ok() == Some(expected) {
            return Ok(());
        }
    }
    Err(format!("delegate signature does not recover {expected}"))
}

casttest!(safe_service_rejects_non_checksum_proposal_addresses, async |_prj, _cmd| {
    // Cast serializes parsed Address values as checksums, so send malformed wire payloads
    // directly to the strict service to cover the validation boundary a remote client crosses.
    let safe = address!("1111111111111111111111111111111111111111");
    let target = address!("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed");
    let gas_token = address!("fB6916095ca1df60bB79Ce92cE3Ea74c37c5d359");
    let refund_receiver = address!("52908400098527886E0F7030069857D2E4169EE7");
    let client = reqwest::Client::new();

    for (field, address, malformed) in [
        ("to", target, checksum(target).to_ascii_lowercase()),
        ("gasToken", gas_token, checksum(gas_token).to_ascii_lowercase()),
        ("refundReceiver", refund_receiver, checksum(refund_receiver).to_ascii_lowercase()),
    ] {
        assert_ne!(malformed, checksum(address), "test address must contain checksum casing");
        let expected = ExpectedTransaction { safe, target, nonce: 0, hash: B256::ZERO };
        let service = spawn_strict_safe_service(vec![ANVIL_OWNER], 31337, expected).await;
        let mut body = json!({
            "to": checksum(target),
            "value": "0",
            "data": "0x",
            "operation": 0,
            "safeTxGas": "0",
            "baseGas": "0",
            "gasPrice": "0",
            "gasToken": checksum(gas_token),
            "refundReceiver": checksum(refund_receiver),
            "nonce": "0",
            "contractTransactionHash": B256::ZERO,
            "sender": checksum(ANVIL_OWNER),
            "signature": "0x",
        });
        body[field] = Value::String(malformed.clone());

        let url = format!(
            "{}/api/v2/safes/{}/multisig-transactions/",
            service.endpoint(),
            checksum(safe)
        );
        let response =
            client.post(url).bearer_auth(SAFE_SERVICE_API_KEY).json(&body).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<Value>().await.unwrap(),
            json!({"detail": format!("{field} is not checksummed: {malformed}")})
        );

        let state = service.state.lock().unwrap();
        assert!(state.transaction.is_none(), "rejected proposal mutated transaction state");
        assert!(state.requests.is_empty(), "rejected proposal recorded a request");
    }
});

casttest!(safe_commands_are_exposed, |_prj, cmd| {
    let output =
        cmd.cast_fuse().args(["safe", "--help"]).assert_success().get_output().stdout_lossy();
    for command in [
        "create",
        "add-delegate",
        "list-delegates",
        "remove-delegate",
        "propose",
        "sign",
        "simulate",
        "execute",
    ] {
        assert!(output.contains(command), "expected `cast safe {command}` in help:\n{output}");
    }
});

casttest!(safe_signing_commands_support_hardware_wallets, |_prj, cmd| {
    for command in ["create", "add-delegate", "remove-delegate", "propose", "sign", "execute"] {
        let output = cmd
            .cast_fuse()
            .args(["safe", command, "--help"])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(output.contains("--ledger"), "expected Ledger support in help:\n{output}");
        assert!(output.contains("--trezor"), "expected Trezor support in help:\n{output}");
    }
});

casttest!(safe_onchain_commands_support_tempo_transaction_options, |_prj, cmd| {
    for command in ["create", "execute"] {
        let output = cmd
            .cast_fuse()
            .args(["safe", command, "--help"])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(
            output.contains("--tempo.fee-token"),
            "expected Tempo fee-token support in help:\n{output}"
        );
        assert!(
            output.contains("--tempo.nonce-key"),
            "expected Tempo nonce-key support in help:\n{output}"
        );
    }
});

casttest!(safe_create_honors_transaction_options, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let singleton = address!("1111111111111111111111111111111111111111");
    let factory = address!("2222222222222222222222222222222222222222");

    api.anvil_set_code(singleton, "0x00".parse().unwrap()).await.unwrap();
    // mstore(singleton); emit ProxyCreation(proxy, singleton); return proxy.
    api.anvil_set_code(
        factory,
        "0x7311111111111111111111111111111111111111115f527333333333333333333333333333333333333333337f4f51faf6c4561ff95f067657e43439f0f856d97c04d9ec9070a6199ad418e23560205fa27333333333333333333333333333333333333333335f5260205ff3"
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();

    let owner = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    let singleton = singleton.to_string();
    let factory = factory.to_string();
    let common_args = [
        "safe",
        "create",
        owner,
        "--singleton",
        &singleton,
        "--factory",
        &factory,
        "--fallback-handler",
        "0x0000000000000000000000000000000000000000",
        "--private-key",
        ANVIL_KEY,
        "--rpc-url",
        &rpc,
    ];

    cmd.cast_fuse().args(common_args).arg("--legacy").assert_success();
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transaction = block.transactions.as_transactions().unwrap().last().unwrap();
    assert_eq!(transaction.ty(), 0);

    cmd.cast_fuse()
        .args(common_args)
        .args([
            "--access-list",
            r#"[{"address":"0x4444444444444444444444444444444444444444","storageKeys":["0x5555555555555555555555555555555555555555555555555555555555555555"]}]"#,
        ])
        .assert_success();
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transaction = block.transactions.as_transactions().unwrap().last().unwrap();
    let access_list = transaction.access_list().expect("explicit access list");
    assert_eq!(access_list.len(), 1);
    assert_eq!(access_list[0].address, address!("4444444444444444444444444444444444444444"));
    assert_eq!(
        access_list[0].storage_keys,
        [b256!("5555555555555555555555555555555555555555555555555555555555555555")]
    );
});

casttest!(safe_v1_4_1_lifecycle_uses_stateful_service, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    api.anvil_set_code(
        SAFE_L2_V1_4_1,
        fixture_runtime(
            include_bytes!("../fixtures/safe/v1.4.1/SafeL2.runtime.bin"),
            SAFE_L2_V1_4_1_RUNTIME_LEN,
            SAFE_L2_V1_4_1_RUNTIME_HASH,
        ),
    )
    .await
    .unwrap();
    api.anvil_set_code(
        SAFE_PROXY_FACTORY_V1_4_1,
        fixture_runtime(
            include_bytes!("../fixtures/safe/v1.4.1/SafeProxyFactory.runtime.bin"),
            SAFE_PROXY_FACTORY_V1_4_1_RUNTIME_LEN,
            SAFE_PROXY_FACTORY_V1_4_1_RUNTIME_HASH,
        ),
    )
    .await
    .unwrap();
    api.anvil_set_code(
        SIMULATE_TX_ACCESSOR_V1_4_1,
        fixture_runtime(
            include_bytes!("../fixtures/safe/v1.4.1/SimulateTxAccessor.runtime.bin"),
            SIMULATE_TX_ACCESSOR_V1_4_1_RUNTIME_LEN,
            SIMULATE_TX_ACCESSOR_V1_4_1_RUNTIME_HASH,
        ),
    )
    .await
    .unwrap();

    let target = address!("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed");
    // Increment slot zero and return msg.sender. The counter distinguishes one execution from
    // repeated calls while the return value keeps the simulation assertion observable.
    api.anvil_set_code(target, "0x6000546001016000553360005260206000f3".parse().unwrap())
        .await
        .unwrap();

    let owners = vec![ANVIL_OWNER, ANVIL_OWNER_2];
    let owner_1 = ANVIL_OWNER.to_string();
    let owner_2 = ANVIL_OWNER_2.to_string();
    let target_arg = target.to_string();
    let create_output = cmd
        .cast_fuse()
        .args([
            "safe",
            "create",
            &owner_1,
            &owner_2,
            "--threshold",
            "2",
            "--singleton",
            &SAFE_L2_V1_4_1.to_string(),
            "--factory",
            &SAFE_PROXY_FACTORY_V1_4_1.to_string(),
            "--fallback-handler",
            "0x0000000000000000000000000000000000000000",
            "--private-key",
            ANVIL_KEY,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let safe: Address = create_output
        .lines()
        .rev()
        .find_map(|line| line.trim().parse().ok())
        .expect("cast safe create did not print a Safe address");
    let safe_contract = TestSafe::new(safe, &provider);
    assert!(!provider.get_code_at(safe).await.unwrap().is_empty());
    assert_eq!(safe_contract.getOwners().call().await.unwrap(), owners);
    assert_eq!(safe_contract.getThreshold().call().await.unwrap(), U256::from(2));
    assert_eq!(safe_contract.nonce().call().await.unwrap(), U256::ZERO);

    let calculated_hash = safe_contract
        .getTransactionHash(
            target,
            U256::ZERO,
            Bytes::new(),
            0,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            Address::ZERO,
            Address::ZERO,
            U256::ZERO,
        )
        .call()
        .await
        .unwrap();
    let service = spawn_strict_safe_service(
        owners,
        31337,
        ExpectedTransaction { safe, target, nonce: 0, hash: calculated_hash },
    )
    .await;

    let safe_arg = safe.to_string();
    let delegate = ANVIL_OWNER_3;
    let delegate_arg = delegate.to_string();
    let service_args = [
        "--service-url",
        service.endpoint(),
        "--api-key",
        SAFE_SERVICE_API_KEY,
        "--rpc-url",
        rpc.as_str(),
    ];

    cmd.cast_fuse()
        .args([
            "--json",
            "safe",
            "add-delegate",
            &safe_arg,
            &delegate_arg,
            "--label",
            "integration delegate",
            "--private-key",
            ANVIL_KEY,
        ])
        .args(service_args)
        .assert_json_stdout(json_envelope(json!(checksum(delegate))));
    cmd.cast_fuse()
        .args(["--json", "safe", "list-delegates", &safe_arg])
        .args(service_args)
        .assert_json_stdout(json_envelope(json!([{
            "safe": safe,
            "delegate": delegate,
            "delegator": ANVIL_OWNER,
            "label": "integration delegate",
        }])));

    let proposal_output = cmd
        .cast_fuse()
        .args(["safe", "propose", &safe_arg, &target_arg, "--private-key", ANVIL_KEY_3])
        .args(service_args)
        .assert_success()
        .get_output()
        .stdout_lossy();
    let safe_tx_hash: B256 = proposal_output
        .lines()
        .rev()
        .find_map(|line| line.trim().parse().ok())
        .expect("cast safe propose did not print a transaction hash");
    assert_eq!(safe_tx_hash, calculated_hash);
    service.assert_delegate_proposal(delegate, safe_tx_hash);

    let safe_tx_hash_arg = safe_tx_hash.to_string();
    cmd.cast_fuse()
        .args(["safe", "sign", &safe_arg, &safe_tx_hash_arg, "--private-key", ANVIL_KEY])
        .args(service_args)
        .assert_success();
    cmd.cast_fuse()
        .args(["safe", "sign", &safe_arg, &safe_tx_hash_arg, "--private-key", ANVIL_KEY_2])
        .args(service_args)
        .assert_success();

    let expected_return_data = hex::encode_prefixed(safe.into_word());
    let accessor_arg = SIMULATE_TX_ACCESSOR_V1_4_1.to_string();
    cmd.cast_fuse()
        .args([
            "--json",
            "safe",
            "simulate",
            &safe_arg,
            &safe_tx_hash_arg,
            "--from",
            &delegate_arg,
            "--accessor",
            &accessor_arg,
        ])
        .args(service_args)
        .assert_json_stdout(json_envelope(json!({
            "safeTxHash": safe_tx_hash,
            "success": true,
            "gasUsed": "[..]",
            "returnData": expected_return_data,
        })));
    assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), U256::ZERO);

    let execution_output = cmd
        .cast_fuse()
        .args(["safe", "execute", &safe_arg, &safe_tx_hash_arg, "--private-key", ANVIL_KEY])
        .args(service_args)
        .assert_success()
        .get_output()
        .stdout_lossy();
    let execution_hash: B256 =
        execution_output.trim().parse().expect("missing outer transaction hash");
    let receipt = provider
        .get_transaction_receipt(execution_hash)
        .await
        .unwrap()
        .expect("Safe execution receipt was not mined");
    assert!(receipt.status());
    assert_eq!(receipt.transaction_hash, execution_hash);
    let matching_events = receipt
        .logs()
        .iter()
        .filter(|log| log.address() == safe)
        .filter_map(|log| TestSafe::ExecutionSuccess::decode_log(&log.inner).ok())
        .filter(|event| event.txHash == safe_tx_hash)
        .collect::<Vec<_>>();
    assert_eq!(matching_events.len(), 1, "expected one matching ExecutionSuccess event");
    assert_eq!(matching_events[0].payment, U256::ZERO);
    assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), U256::ONE);
    assert_eq!(safe_contract.nonce().call().await.unwrap(), U256::ONE);

    cmd.cast_fuse()
        .args([
            "--json",
            "safe",
            "remove-delegate",
            &safe_arg,
            &delegate_arg,
            "--private-key",
            ANVIL_KEY,
        ])
        .args(service_args)
        .assert_json_stdout(json_envelope(json!(checksum(delegate))));
    cmd.cast_fuse()
        .args(["--json", "safe", "list-delegates", &safe_arg])
        .args(service_args)
        .assert_json_stdout(json_envelope(json!([])));
    service.assert_lifecycle_complete(
        &[
            RequestEvent::AddDelegate { safe, delegate, delegator: ANVIL_OWNER },
            RequestEvent::ListDelegates { safe },
            RequestEvent::NonceLookup { safe },
            RequestEvent::Propose { safe, sender: delegate, hash: safe_tx_hash },
            RequestEvent::GetTransaction { version: 1, hash: safe_tx_hash },
            RequestEvent::Confirm { hash: safe_tx_hash, owner: ANVIL_OWNER },
            RequestEvent::GetTransaction { version: 1, hash: safe_tx_hash },
            RequestEvent::Confirm { hash: safe_tx_hash, owner: ANVIL_OWNER_2 },
            RequestEvent::GetTransaction { version: 2, hash: safe_tx_hash },
            RequestEvent::GetTransaction { version: 2, hash: safe_tx_hash },
            RequestEvent::RemoveDelegate { safe, delegate, delegator: ANVIL_OWNER },
            RequestEvent::ListDelegates { safe },
        ],
        delegate,
    );
});

casttest!(safe_execute_rejects_approved_hash_confirmation, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    // Return a zero bytes32 word for both getTransactionHash() and nonce().
    api.anvil_set_code(safe, "0x600060005260206000f3".parse().unwrap()).await.unwrap();
    let mut transaction = safe_transaction(safe, 0);
    let mut approved_hash_signature = vec![0u8; 65];
    approved_hash_signature[..32].copy_from_slice(ANVIL_OWNER.into_word().as_slice());
    approved_hash_signature[64] = 1;
    transaction["confirmations"] = json!([{
        "owner": ANVIL_OWNER,
        "signature": hex::encode_prefixed(approved_hash_signature),
    }]);
    let service = spawn_safe_service(transaction).await;
    cmd.cast_fuse()
        .args([
            "safe",
            "execute",
            &safe.to_string(),
            &B256::ZERO.to_string(),
            "--service-url",
            service.endpoint(),
            "--rpc-url",
            &rpc,
            "--private-key",
            ANVIL_KEY,
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Safe transaction: 0x0000000000000000000000000000000000000000000000000000000000000000
  Safe:            0x1111111111111111111111111111111111111111
  To:              0x0000000000000000000000000000000000000000
  Value:           0
  Operation:       0 (CALL)
  Safe tx gas:     0
  Base gas:        0
  Gas price:       0
  Gas token:       0x0000000000000000000000000000000000000000
  Refund receiver: 0x0000000000000000000000000000000000000000
  Nonce:           0
  Data:            0x
Error: approved-hash signatures (v = 1) are not supported by `cast safe execute`

"#]]);
});

casttest!(safe_execute_rejects_future_nonce, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    api.anvil_set_code(safe, "0x600060005260206000f3".parse().unwrap()).await.unwrap();
    let mut transaction = safe_transaction(safe, 0);
    transaction["nonce"] = json!("1");
    let service = spawn_safe_service(transaction).await;
    cmd.cast_fuse()
        .args([
            "safe",
            "execute",
            &safe.to_string(),
            &B256::ZERO.to_string(),
            "--service-url",
            service.endpoint(),
            "--rpc-url",
            &rpc,
            "--private-key",
            ANVIL_KEY,
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Safe transaction nonce 1 does not match current Safe nonce 0

"#]]);
});

casttest!(safe_execute_rejects_stale_nonce, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    // Return one for nonce() and zero for getTransactionHash().
    api.anvil_set_code(
        safe,
        "0x5f3560e01c63affed0e0146015575f5f5260205ff35b60015f5260205ff3".parse().unwrap(),
    )
    .await
    .unwrap();
    let transaction = safe_transaction(safe, 0);
    let service = spawn_safe_service(transaction).await;
    cmd.cast_fuse()
        .args([
            "safe",
            "execute",
            &safe.to_string(),
            &B256::ZERO.to_string(),
            "--service-url",
            service.endpoint(),
            "--rpc-url",
            &rpc,
            "--private-key",
            ANVIL_KEY,
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Safe transaction nonce 0 does not match current Safe nonce 1

"#]]);
});

casttest!(safe_execute_rejects_approved_hash_with_stale_or_future_nonce, async |_prj, cmd| {
    // A v = 1 confirmation must not make a transaction with a stale or future nonce
    // executable. Use a fresh node for each case so the no-broadcast assertions are
    // independent and a regression cannot hide behind state from the other case.
    for (nonce, current_nonce, safe_code, expected_error) in [
        (
            1u64,
            0u64,
            "0x600060005260206000f3",
            "Error: Safe transaction nonce 1 does not match current Safe nonce 0\n",
        ),
        (
            0u64,
            1u64,
            "0x5f3560e01c63affed0e0146015575f5f5260205ff35b60015f5260205ff3",
            "Error: Safe transaction nonce 0 does not match current Safe nonce 1\n",
        ),
    ] {
        let (api, handle) = anvil::spawn(NodeConfig::test()).await;
        let rpc = handle.http_endpoint();
        let provider = handle.http_provider();
        let safe = address!("1111111111111111111111111111111111111111");
        api.anvil_set_code(safe, safe_code.parse().unwrap()).await.unwrap();

        let initial_block = provider.get_block_number().await.unwrap();
        let initial_sender_nonce = provider.get_transaction_count(ANVIL_OWNER).await.unwrap();
        let mut transaction = safe_transaction(safe, 0);
        transaction["nonce"] = json!(nonce.to_string());
        let mut approved_hash_signature = vec![0u8; 65];
        approved_hash_signature[..32].copy_from_slice(ANVIL_OWNER.into_word().as_slice());
        approved_hash_signature[64] = 1;
        transaction["confirmations"] = json!([{
            "owner": ANVIL_OWNER,
            "signature": hex::encode_prefixed(approved_hash_signature),
        }]);
        let service = spawn_safe_service(transaction).await;

        let safe_arg = safe.to_string();
        let hash_arg = B256::ZERO.to_string();
        cmd.cast_fuse()
            .args([
                "safe",
                "execute",
                &safe_arg,
                &hash_arg,
                "--service-url",
                service.endpoint(),
                "--rpc-url",
                &rpc,
                "--private-key",
                ANVIL_KEY,
            ])
            .assert_failure()
            .stdout_eq("")
            .stderr_eq(expected_error);

        // Nonce validation happens before eth_sendTransaction; prove that no outer transaction
        // was submitted even though the service supplied an approved-hash confirmation.
        assert_eq!(provider.get_block_number().await.unwrap(), initial_block);
        assert_eq!(
            provider.get_transaction_count(ANVIL_OWNER).await.unwrap(),
            initial_sender_nonce
        );
        assert_eq!(
            TestSafe::new(safe, &provider).nonce().call().await.unwrap(),
            U256::from(current_nonce)
        );
        let latest =
            provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
        assert!(latest.transactions.as_transactions().unwrap().is_empty());
    }
});

casttest!(safe_execute_packs_mixed_p256_confirmations, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let safe = address!("1111111111111111111111111111111111111111");
    // Return zero for hash/nonce calls and emit ExecutionSuccess(bytes32(0), 0).
    api.anvil_set_code(
        safe,
        "0x5f7f442e715f626346e8c54381002da614f62bee8d27386535b2521ec8540898556e60205fa260205ff3"
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();

    let p256_owner = Address::repeat_byte(1);
    let eoa_owner = Address::repeat_byte(2);
    let contract_owner = Address::repeat_byte(3);
    let p256_payload = [4; 128];
    let contract_payload = [5, 6, 7];

    let mut p256_signature = Vec::with_capacity(193);
    p256_signature.extend_from_slice(p256_owner.into_word().as_slice());
    p256_signature.extend_from_slice(&U256::from(65).to_be_bytes::<32>());
    p256_signature.push(2);
    p256_signature.extend_from_slice(&p256_payload);

    let mut eoa_signature = vec![2; 65];
    eoa_signature[64] = 27;

    let mut contract_signature = Vec::with_capacity(100);
    contract_signature.extend_from_slice(contract_owner.into_word().as_slice());
    contract_signature.extend_from_slice(&U256::from(65).to_be_bytes::<32>());
    contract_signature.push(0);
    contract_signature.extend_from_slice(&U256::from(contract_payload.len()).to_be_bytes::<32>());
    contract_signature.extend_from_slice(&contract_payload);

    let mut service_transaction = safe_transaction(safe, 0);
    service_transaction["confirmations"] = json!([
        {
            "owner": contract_owner,
            "signature": hex::encode_prefixed(&contract_signature),
        },
        {
            "owner": eoa_owner,
            "signature": hex::encode_prefixed(&eoa_signature),
        },
        {
            "owner": p256_owner,
            "signature": hex::encode_prefixed(&p256_signature),
        },
    ]);
    let service = spawn_safe_service(service_transaction).await;

    cmd.cast_fuse()
        .args([
            "safe",
            "execute",
            &safe.to_string(),
            &B256::ZERO.to_string(),
            "--service-url",
            service.endpoint(),
            "--rpc-url",
            &rpc,
            "--private-key",
            ANVIL_KEY,
        ])
        .assert_success();

    let mut expected_signatures = Vec::with_capacity(358);
    expected_signatures.extend_from_slice(p256_owner.into_word().as_slice());
    expected_signatures.extend_from_slice(&U256::from(195).to_be_bytes::<32>());
    expected_signatures.push(2);
    expected_signatures.extend_from_slice(&eoa_signature);
    expected_signatures.extend_from_slice(contract_owner.into_word().as_slice());
    expected_signatures.extend_from_slice(&U256::from(323).to_be_bytes::<32>());
    expected_signatures.push(0);
    expected_signatures.extend_from_slice(&p256_payload);
    expected_signatures.extend_from_slice(&U256::from(contract_payload.len()).to_be_bytes::<32>());
    expected_signatures.extend_from_slice(&contract_payload);

    let expected_calldata = TestSafe::execTransactionCall {
        to: Address::ZERO,
        value: U256::ZERO,
        data: Bytes::new(),
        operation: 0,
        safeTxGas: U256::ZERO,
        baseGas: U256::ZERO,
        gasPrice: U256::ZERO,
        gasToken: Address::ZERO,
        refundReceiver: Address::ZERO,
        signatures: expected_signatures.into(),
    }
    .abi_encode();
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let submitted = block.transactions.as_transactions().unwrap().last().unwrap();
    assert_eq!(submitted.input(), expected_calldata.as_slice());
});

casttest!(safe_service_mutations_emit_json_envelopes, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    let delegate = address!("2222222222222222222222222222222222222222");
    let target = address!("3333333333333333333333333333333333333333");
    api.anvil_set_code(safe, "0x5f5f5260205ff3".parse().unwrap()).await.unwrap();
    let service = spawn_safe_service(safe_transaction(safe, 0)).await;
    let safe = safe.to_string();
    let delegate = delegate.to_string();
    let target = target.to_string();
    let safe_tx_hash = B256::ZERO.to_string();
    let signer_args = [
        "--service-url",
        service.endpoint(),
        "--rpc-url",
        rpc.as_str(),
        "--private-key",
        ANVIL_KEY,
    ];

    cmd.cast_fuse()
        .args(["--json", "safe", "add-delegate", &safe, &delegate, "--label", "test"])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(delegate)));

    cmd.cast_fuse()
        .args(["--json", "safe", "remove-delegate", &safe, &delegate])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(delegate)));

    cmd.cast_fuse()
        .args(["--json", "safe", "propose", &safe, &target, "--nonce", "0"])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(B256::ZERO)));

    let signer: PrivateKeySigner = ANVIL_KEY.parse().unwrap();
    let mut signature = signer.sign_message(B256::ZERO.as_slice()).await.unwrap().as_bytes();
    signature[64] += 4;
    let signature = hex::encode_prefixed(signature);
    cmd.cast_fuse()
        .args(["--json", "safe", "sign", &safe, &safe_tx_hash])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(signature)));
});

casttest!(safe_list_delegates_follows_pagination, async |_prj, cmd| {
    let safe = address!("1111111111111111111111111111111111111111");
    let first = json!({
        "safe": safe,
        "delegate": address!("2222222222222222222222222222222222222222"),
        "delegator": address!("3333333333333333333333333333333333333333"),
        "label": "first",
    });
    let second = json!({
        "safe": safe,
        "delegate": address!("4444444444444444444444444444444444444444"),
        "delegator": address!("5555555555555555555555555555555555555555"),
        "label": "second",
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let next = format!("{endpoint}/api/v2/delegates/?page=2");
    let first_response = first.clone();
    let second_response = second.clone();
    let router = Router::new().fallback(move |RawQuery(query): RawQuery| {
        let first = first_response.clone();
        let second = second_response.clone();
        let next = next.clone();
        async move {
            let is_second_page =
                query.as_deref().is_some_and(|query| query.split('&').any(|pair| pair == "page=2"));
            Json(if is_second_page {
                json!({ "count": 2, "next": null, "previous": null, "results": [second] })
            } else {
                json!({ "count": 2, "next": next, "previous": null, "results": [first] })
            })
        }
    });
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let server = TestServerHandle { endpoint, task };

    cmd.cast_fuse()
        .args([
            "--json",
            "safe",
            "list-delegates",
            &safe.to_string(),
            "--service-url",
            server.endpoint(),
        ])
        .assert_json_stdout(json_envelope(json!([first, second])));
});

casttest!(safe_list_delegates_rejects_external_next_page, async |_prj, cmd| {
    let service = spawn_safe_service(json!({
        "count": 1,
        "next": "http://127.0.0.1:1/collect-api-key",
        "previous": null,
        "results": [],
    }))
    .await;
    cmd.cast_fuse()
        .args([
            "safe",
            "list-delegates",
            "0x1111111111111111111111111111111111111111",
            "--service-url",
            service.endpoint(),
        ])
        .assert_failure()
        .stdout_eq("")
        .stderr_eq(str![[r#"
Error: delegate pagination URL points outside the Transaction Service endpoint: http://127.0.0.1:1/collect-api-key

"#]]);
});

casttest!(safe_sign_rejects_service_selected_safe, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let expected = address!("1111111111111111111111111111111111111111");
    let malicious = address!("2222222222222222222222222222222222222222");
    api.anvil_set_code(malicious, "0x5f5f5260205ff3".parse().unwrap()).await.unwrap();
    let service = spawn_safe_service(safe_transaction(malicious, 0)).await;
    let expected_arg = expected.to_string();
    let safe_tx_hash = B256::ZERO.to_string();

    cmd.cast_fuse()
        .args([
            "safe",
            "sign",
            &expected_arg,
            &safe_tx_hash,
            "--service-url",
            service.endpoint(),
            "--rpc-url",
            &rpc,
            "--private-key",
            ANVIL_KEY,
        ])
        .assert_failure()
        .stdout_eq("")
        .stderr_eq(format!(
            "Error: Transaction Service returned Safe {malicious}, expected {expected}\n"
        ));
});

casttest!(safe_simulation_requires_executor, |_prj, cmd| {
    let safe = address!("1111111111111111111111111111111111111111").to_string();
    let safe_tx_hash = B256::ZERO.to_string();

    cmd.cast_fuse();
    cmd.unset_env("ETH_FROM");
    cmd.args(["safe", "simulate", &safe, &safe_tx_hash, "--rpc-url", "http://127.0.0.1:1"])
        .assert_failure()
        .stdout_eq("")
        .stderr_eq(str![[r#"
error: the following required arguments were not provided:
  --from <ADDRESS>

Usage: cast[..] safe simulate --from <ADDRESS> --rpc-url <URL> <SAFE> <SAFE_TX_HASH>

For more information, try '--help'.

"#]]);
});

casttest!(safe_simulation_uses_executor_context, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    let executor = address!("3333333333333333333333333333333333333333");
    let accessor = address!("4444444444444444444444444444444444444444");
    // Return a zero transaction hash, then require `tx.origin` to be `executor` and return it as
    // the simulated call result.
    api.anvil_set_code(safe, SIMULATION_SAFE_CODE.parse().unwrap()).await.unwrap();
    api.anvil_set_code(accessor, "0x00".parse().unwrap()).await.unwrap();
    let safe_arg = safe.to_string();
    let executor_arg = executor.to_string();
    let accessor_arg = accessor.to_string();
    let safe_tx_hash = B256::ZERO.to_string();

    for operation in [0, 1] {
        let service = spawn_safe_service(safe_transaction(safe, operation)).await;
        cmd.cast_fuse()
            .args([
                "--json",
                "safe",
                "simulate",
                &safe_arg,
                &safe_tx_hash,
                "--from",
                &executor_arg,
                "--accessor",
                &accessor_arg,
                "--service-url",
                service.endpoint(),
                "--rpc-url",
                &rpc,
            ])
            .assert_json_stdout(json_envelope(json!({
                "safeTxHash": B256::ZERO,
                "success": true,
                "gasUsed": "42",
                "returnData": "0x0000000000000000000000003333333333333333333333333333333333333333",
            })));
    }
});

casttest!(safe_simulation_rejects_reimbursed_transaction, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    let executor = address!("3333333333333333333333333333333333333333");
    let accessor = address!("4444444444444444444444444444444444444444");
    api.anvil_set_code(safe, SIMULATION_SAFE_CODE.parse().unwrap()).await.unwrap();
    api.anvil_set_code(accessor, "0x00".parse().unwrap()).await.unwrap();
    let mut transaction = safe_transaction(safe, 0);
    transaction["gasPrice"] = json!("1");
    let service = spawn_safe_service(transaction).await;

    cmd.cast_fuse()
        .args([
            "safe",
            "simulate",
            &safe.to_string(),
            &B256::ZERO.to_string(),
            "--from",
            &executor.to_string(),
            "--accessor",
            &accessor.to_string(),
            "--service-url",
            service.endpoint(),
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .stdout_eq("")
        .stderr_eq(str![[r#"
Safe transaction: 0x0000000000000000000000000000000000000000000000000000000000000000
  Safe:            0x1111111111111111111111111111111111111111
  To:              0x0000000000000000000000000000000000000000
  Value:           0
  Operation:       0 (CALL)
  Safe tx gas:     0
  Base gas:        0
  Gas price:       1
  Gas token:       0x0000000000000000000000000000000000000000
  Refund receiver: 0x0000000000000000000000000000000000000000
  Nonce:           0
  Data:            0x
Error: cannot simulate reimbursed Safe transactions (gasPrice > 0): SimulateTxAccessor does not enforce safeTxGas

"#]]);
});

casttest!(safe_simulation_call_target_checks_origin_and_sender, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    api.anvil_set_code(
        SAFE_L2_V1_4_1,
        fixture_runtime(
            include_bytes!("../fixtures/safe/v1.4.1/SafeL2.runtime.bin"),
            SAFE_L2_V1_4_1_RUNTIME_LEN,
            SAFE_L2_V1_4_1_RUNTIME_HASH,
        ),
    )
    .await
    .unwrap();
    api.anvil_set_code(
        SAFE_PROXY_FACTORY_V1_4_1,
        fixture_runtime(
            include_bytes!("../fixtures/safe/v1.4.1/SafeProxyFactory.runtime.bin"),
            SAFE_PROXY_FACTORY_V1_4_1_RUNTIME_LEN,
            SAFE_PROXY_FACTORY_V1_4_1_RUNTIME_HASH,
        ),
    )
    .await
    .unwrap();
    api.anvil_set_code(
        SIMULATE_TX_ACCESSOR_V1_4_1,
        fixture_runtime(
            include_bytes!("../fixtures/safe/v1.4.1/SimulateTxAccessor.runtime.bin"),
            SIMULATE_TX_ACCESSOR_V1_4_1_RUNTIME_LEN,
            SIMULATE_TX_ACCESSOR_V1_4_1_RUNTIME_HASH,
        ),
    )
    .await
    .unwrap();

    let owner_1 = ANVIL_OWNER.to_string();
    let owner_2 = ANVIL_OWNER_2.to_string();
    let create_output = cmd
        .cast_fuse()
        .args([
            "safe",
            "create",
            &owner_1,
            &owner_2,
            "--threshold",
            "2",
            "--singleton",
            &SAFE_L2_V1_4_1.to_string(),
            "--factory",
            &SAFE_PROXY_FACTORY_V1_4_1.to_string(),
            "--fallback-handler",
            "0x0000000000000000000000000000000000000000",
            "--private-key",
            ANVIL_KEY,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let safe: Address = create_output
        .lines()
        .rev()
        .find_map(|line| line.trim().parse().ok())
        .expect("cast safe create did not print a Safe address");

    // The target rejects either context mismatch, then returns both context values for the
    // simulation assertion. The Safe address is patched into the CALLER comparison after create.
    let target = address!("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed");
    let mut target_code = hex!(
        "733c44cdddb6a900fa2b585dd299e03d12fa4293bc321415604157730000000000000000000000000000000000000000331415604157325f523360205260405ff35b5f5ffd"
    );
    target_code[28..48].copy_from_slice(safe.as_slice());
    api.anvil_set_code(target, Bytes::copy_from_slice(&target_code)).await.unwrap();

    let safe_contract = TestSafe::new(safe, &provider);
    let safe_tx_hash = safe_contract
        .getTransactionHash(
            target,
            U256::ZERO,
            Bytes::new(),
            0,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            Address::ZERO,
            Address::ZERO,
            U256::ZERO,
        )
        .call()
        .await
        .unwrap();
    let mut transaction = safe_transaction(safe, 0);
    transaction["to"] = json!(target);
    transaction["safeTxHash"] = json!(safe_tx_hash);
    let service = spawn_safe_service(transaction).await;

    let executor = ANVIL_OWNER_3;
    let mut expected_return_data = Vec::with_capacity(2 * Address::len_bytes() * 2);
    expected_return_data.extend_from_slice(executor.into_word().as_slice());
    expected_return_data.extend_from_slice(safe.into_word().as_slice());
    let expected_return_data = hex::encode_prefixed(expected_return_data);
    cmd.cast_fuse()
        .args([
            "--json",
            "safe",
            "simulate",
            &safe.to_string(),
            &safe_tx_hash.to_string(),
            "--from",
            &executor.to_string(),
            "--accessor",
            &SIMULATE_TX_ACCESSOR_V1_4_1.to_string(),
            "--service-url",
            service.endpoint(),
            "--rpc-url",
            &rpc,
        ])
        .assert_json_stdout(json_envelope(json!({
            "safeTxHash": safe_tx_hash,
            "success": true,
            "gasUsed": "[..]",
            "returnData": expected_return_data,
        })));
});
