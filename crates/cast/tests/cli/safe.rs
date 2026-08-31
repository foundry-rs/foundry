use alloy_consensus::Transaction;
use alloy_dyn_abi::TypedData;
use alloy_eips::Typed2718;
use alloy_primitives::{Address, B256, Bytes, Signature, U256, address, b256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::BlockNumberOrTag;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::SolCall;
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
const ANVIL_OWNER: Address = address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
const ANVIL_OWNER_2: Address = address!("70997970C51812dc3A010C7d01b50e0d17dc79C8");
const ANVIL_OWNER_3: Address = address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC");
const SAFE_L2_V1_4_1: Address = address!("29fcB43b46531BcA003ddC8FCB67FFE91900C762");
const SAFE_PROXY_FACTORY_V1_4_1: Address = address!("4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67");
const SIMULATE_TX_ACCESSOR_V1_4_1: Address = address!("3d4BA2E0884aa488718476ca2FB8Efc291A46199");
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

async fn spawn_safe_service(response: Value) -> String {
    let router = Router::new().fallback(move || {
        let response = response.clone();
        async move { Json(response) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    endpoint
}

fn json_envelope(data: Value) -> String {
    serde_json::to_string(&JsonEnvelope::success(data)).unwrap()
}

fn fixture_runtime(source: &str) -> Bytes {
    source.split_whitespace().collect::<String>().parse().unwrap()
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
    confirmations: Vec<StrictConfirmation>,
    executed: bool,
}

#[derive(Debug, Default)]
struct StrictServiceState {
    chain_id: u64,
    api_key: String,
    owners: Vec<Address>,
    safe: Option<Address>,
    expected_target: Option<Address>,
    expected_hash: Option<B256>,
    expected_nonce: Option<u64>,
    delegates: Vec<(Address, Address, String)>,
    transaction: Option<StrictTransaction>,
    requests: Vec<String>,
}

/// A small, stateful Transaction Service double used by the lifecycle test.
///
/// It deliberately accepts only the routes used by Cast and checks the wire-level
/// representation, so a test cannot pass by talking to a permissive catch-all.
struct SafeServiceHandle {
    endpoint: String,
    state: Arc<Mutex<StrictServiceState>>,
    task: JoinHandle<()>,
}

impl SafeServiceHandle {
    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn set_safe(&self, safe: Address) {
        let mut state = self.state.lock().unwrap();
        assert!(state.safe.replace(safe).is_none(), "Safe address was configured twice");
    }

    fn expect_transaction_shape(&self, target: Address, nonce: u64) {
        let mut state = self.state.lock().unwrap();
        state.expected_target = Some(target);
        state.expected_nonce = Some(nonce);
    }

    fn transaction_hash(&self) -> B256 {
        self.state.lock().unwrap().transaction.as_ref().expect("transaction was not proposed").hash
    }

    fn mark_executed(&self) {
        let mut state = self.state.lock().unwrap();
        state.transaction.as_mut().expect("proposal was not submitted").executed = true;
    }

    fn assert_lifecycle_complete(&self) {
        let state = self.state.lock().unwrap();
        assert!(state.delegates.is_empty(), "delegate was not removed: {:?}", state.delegates);
        let transaction = state.transaction.as_ref().expect("transaction was not proposed");
        assert!(transaction.executed, "transaction was not marked executed");
        assert_eq!(transaction.confirmations.len(), state.owners.len());
        assert_eq!(transaction.nonce, state.expected_nonce.unwrap());
        assert!(
            state.requests.iter().any(|request| request == "POST /api/v2/delegates/"),
            "missing delegate registration request: {:?}",
            state.requests
        );
        assert!(
            state.requests.iter().any(|request| request == "GET /api/v2/delegates/"),
            "missing delegate list request: {:?}",
            state.requests
        );
        assert!(
            state.requests.iter().any(|request| request.starts_with("POST /api/v2/safes/")),
            "missing proposal request: {:?}",
            state.requests
        );
        assert!(
            state
                .requests
                .iter()
                .any(|request| request.starts_with("GET /api/v1/multisig-transactions/")),
            "missing v1 transaction request: {:?}",
            state.requests
        );
        assert!(
            state
                .requests
                .iter()
                .any(|request| request.starts_with("POST /api/v1/multisig-transactions/")),
            "missing confirmation request: {:?}",
            state.requests
        );
        assert!(
            state
                .requests
                .iter()
                .any(|request| request.starts_with("GET /api/v2/multisig-transactions/")),
            "missing v2 transaction request: {:?}",
            state.requests
        );
        assert!(
            state.requests.iter().any(|request| request == "DELETE /api/v2/delegates/"),
            "missing delegate removal request: {:?}",
            state.requests
        );
    }
}

impl Drop for SafeServiceHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn_strict_safe_service(owners: Vec<Address>, chain_id: u64) -> SafeServiceHandle {
    let state = Arc::new(Mutex::new(StrictServiceState {
        chain_id,
        api_key: SAFE_SERVICE_API_KEY.to_string(),
        owners,
        ..Default::default()
    }));
    let router = Router::new().fallback(strict_service_handler).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    SafeServiceHandle { endpoint, state, task }
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
    let route = format!("{method_name} {path}");
    if path == "/api/v2/delegates/" {
        match method {
            Method::GET => {
                handle_list_delegates(state, uri)?;
                state.requests.push(route);
                let safe = state.safe.ok_or("Safe address is not configured")?;
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
                handle_add_delegate(state, body)?;
                state.requests.push(route);
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
        handle_proposal(state, suffix, body)?;
        state.requests.push(route);
        return Ok(ServiceResponse::Empty);
    }
    if let Some(safe_path) = path.strip_prefix("/api/v1/safes/")
        && let Some(suffix) = safe_path.strip_suffix("/multisig-transactions/")
    {
        ensure_method(&method, Method::GET, path)?;
        handle_nonce_lookup(state, suffix, uri)?;
        state.requests.push(route);
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
            handle_confirmation(state, hash, body)?;
            state.requests.push(route);
            return Ok(ServiceResponse::Empty);
        }
        if let Some(hash) = tx_path.strip_suffix('/') {
            ensure_method(&method, Method::GET, path)?;
            ensure_no_query(uri)?;
            let transaction = transaction_json(handle_transaction_get(state, hash)?);
            state.requests.push(route);
            return Ok(ServiceResponse::Json(transaction));
        }
    }
    if let Some(tx_path) = path.strip_prefix("/api/v2/multisig-transactions/")
        && let Some(hash) = tx_path.strip_suffix('/')
    {
        ensure_method(&method, Method::GET, path)?;
        ensure_no_query(uri)?;
        let transaction = transaction_json(handle_transaction_get(state, hash)?);
        state.requests.push(route);
        return Ok(ServiceResponse::Json(transaction));
    }
    if let Some(delegate_path) = path.strip_prefix("/api/v2/delegates/")
        && let Some(delegate) = delegate_path.strip_suffix('/')
    {
        if method != Method::DELETE {
            return Err(format!("unexpected method {method_name} for {path}"));
        }
        ensure_no_query(uri)?;
        handle_remove_delegate(state, delegate, body)?;
        state.requests.push("DELETE /api/v2/delegates/".to_string());
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
    let safe = state.safe.ok_or("Safe address is not configured")?;
    let expected = format!("safe={}", checksum(safe));
    if uri.query() != Some(expected.as_str()) {
        return Err(format!("delegate list query must be `{expected}`, got {:?}", uri.query()));
    }
    Ok(())
}

fn handle_add_delegate(state: &mut StrictServiceState, body: Value) -> Result<(), String> {
    let safe = state.safe.ok_or("Safe address is not configured")?;
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
    Ok(())
}

fn handle_remove_delegate(
    state: &mut StrictServiceState,
    path: &str,
    body: Value,
) -> Result<(), String> {
    let safe = state.safe.ok_or("Safe address is not configured")?;
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
    Ok(())
}

fn handle_nonce_lookup(state: &StrictServiceState, path: &str, uri: &Uri) -> Result<(), String> {
    let safe = state.safe.ok_or("Safe address is not configured")?;
    ensure_checksum_path(path, safe, "Safe")?;
    let expected = "executed=false&ordering=-nonce&limit=1";
    if uri.query() != Some(expected) {
        return Err(format!("nonce lookup query must be `{expected}`, got {:?}", uri.query()));
    }
    Ok(())
}

fn handle_proposal(state: &mut StrictServiceState, path: &str, body: Value) -> Result<(), String> {
    let safe = state.safe.ok_or("Safe address is not configured")?;
    ensure_checksum_path(path, safe, "Safe")?;
    if state.transaction.is_some() {
        return Err("only one Safe transaction is supported by this test service".to_string());
    }
    let target = parse_checksum_field(&body, "to")?;
    if let Some(expected_target) = state.expected_target
        && target != expected_target
    {
        return Err(format!("proposal target {target} does not match {expected_target}"));
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
    if let Some(expected_nonce) = state.expected_nonce
        && nonce != expected_nonce
    {
        return Err(format!("proposal nonce {nonce} does not match {expected_nonce}"));
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
    if let Some(expected_hash) = state.expected_hash
        && hash != expected_hash
    {
        return Err(format!("proposal hash {hash} does not match {expected_hash}"));
    }
    let sender = parse_checksum_field(&body, "sender")?;
    if !state.owners.contains(&sender) {
        return Err(format!("proposal sender {sender} is not a Safe owner"));
    }
    let signature = body
        .get("signature")
        .and_then(Value::as_str)
        .ok_or("proposal signature is missing")?
        .to_owned();
    ensure_safe_signature(&signature, hash, sender)?;
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
        confirmations: vec![StrictConfirmation { owner: sender, signature }],
        executed: false,
    });
    Ok(())
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
) -> Result<(), String> {
    let hash =
        path.parse::<B256>().map_err(|error| format!("invalid transaction hash path: {error}"))?;
    let transaction_hash = state.transaction.as_ref().ok_or("transaction was not proposed")?.hash;
    if transaction_hash != hash {
        return Err(format!("requested transaction {hash}, expected {transaction_hash}"));
    }
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
    Ok(())
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
        "isExecuted": transaction.executed,
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

fn decode_signature(value: &str, expected_len: usize) -> Result<Vec<u8>, String> {
    let bytes =
        value.parse::<Bytes>().map_err(|error| format!("invalid signature encoding: {error}"))?;
    if bytes.len() != expected_len {
        return Err(format!("expected {expected_len}-byte signature, got {}", bytes.len()));
    }
    Ok(bytes.to_vec())
}

fn recover_safe_signature(value: &str, hash: B256) -> Result<Address, String> {
    let mut bytes = decode_signature(value, 65)?;
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
    let bytes = decode_signature(value, 65)?;
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
        fixture_runtime(include_str!("../fixtures/safe/v1.4.1/SafeL2.runtime.hex")),
    )
    .await
    .unwrap();
    api.anvil_set_code(
        SAFE_PROXY_FACTORY_V1_4_1,
        fixture_runtime(include_str!("../fixtures/safe/v1.4.1/SafeProxyFactory.runtime.hex")),
    )
    .await
    .unwrap();
    api.anvil_set_code(
        SIMULATE_TX_ACCESSOR_V1_4_1,
        fixture_runtime(include_str!("../fixtures/safe/v1.4.1/SimulateTxAccessor.runtime.hex")),
    )
    .await
    .unwrap();

    let target = address!("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed");
    // Store and return msg.sender. This makes CALL simulation and execution observable.
    api.anvil_set_code(target, "0x338060005560005260206000f3".parse().unwrap()).await.unwrap();

    let owners = vec![ANVIL_OWNER, ANVIL_OWNER_2];
    let service = spawn_strict_safe_service(owners.clone(), 31337).await;
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
    assert_eq!(safe_contract.getOwners().call().await.unwrap(), owners);
    assert_eq!(safe_contract.getThreshold().call().await.unwrap(), U256::from(2));
    service.set_safe(safe);
    service.expect_transaction_shape(target, 0);

    let safe_arg = safe.to_string();
    let delegate = address!("1111111111111111111111111111111111111111");
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
        .assert_json_stdout(json_envelope(json!(delegate)));
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
        .args(["safe", "propose", &safe_arg, &target_arg, "--private-key", ANVIL_KEY])
        .args(service_args)
        .assert_success()
        .get_output()
        .stdout_lossy();
    let safe_tx_hash: B256 = proposal_output
        .lines()
        .rev()
        .find_map(|line| line.trim().parse().ok())
        .expect("cast safe propose did not print a transaction hash");
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
    assert_eq!(safe_tx_hash, calculated_hash);
    assert_eq!(safe_tx_hash, service.transaction_hash());

    let safe_tx_hash_arg = safe_tx_hash.to_string();
    cmd.cast_fuse()
        .args(["safe", "sign", &safe_arg, &safe_tx_hash_arg, "--private-key", ANVIL_KEY_2])
        .args(service_args)
        .assert_success();

    let expected_return_data = format!("0x{:064x}", U256::from_be_slice(safe.as_slice()));
    cmd.cast_fuse()
        .args([
            "--json",
            "safe",
            "simulate",
            &safe_arg,
            &safe_tx_hash_arg,
            "--from",
            &ANVIL_OWNER_3.to_string(),
        ])
        .args([
            "--accessor",
            &SIMULATE_TX_ACCESSOR_V1_4_1.to_string(),
            "--service-url",
            service.endpoint(),
            "--api-key",
            SAFE_SERVICE_API_KEY,
            "--rpc-url",
            rpc.as_str(),
        ])
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
    assert!(execution_output.trim().parse::<B256>().is_ok());
    assert_eq!(
        provider.get_storage_at(target, U256::ZERO).await.unwrap(),
        U256::from_be_slice(safe.as_slice())
    );
    assert_eq!(safe_contract.nonce().call().await.unwrap(), U256::ONE);
    service.mark_executed();

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
        .assert_json_stdout(json_envelope(json!(delegate)));
    cmd.cast_fuse()
        .args(["--json", "safe", "list-delegates", &safe_arg])
        .args(service_args)
        .assert_json_stdout(json_envelope(json!([])));
    service.assert_lifecycle_complete();
});

casttest!(safe_execute_rejects_approved_hash_confirmation, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    // Return a zero bytes32 word for both getTransactionHash() and nonce().
    api.anvil_set_code(safe, "0x600060005260206000f3".parse().unwrap()).await.unwrap();
    let mut transaction = safe_transaction(safe, 0);
    let mut approved_hash_signature = vec![0u8; 65];
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
            &service,
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
            &service,
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
            &service,
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
            &service,
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
    let signer_args =
        ["--service-url", service.as_str(), "--rpc-url", rpc.as_str(), "--private-key", ANVIL_KEY];

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
    let service = format!("http://{}", listener.local_addr().unwrap());
    let next = format!("{service}/api/v2/delegates/?page=2");
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
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    cmd.cast_fuse()
        .args(["--json", "safe", "list-delegates", &safe.to_string(), "--service-url", &service])
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
    let stderr = cmd
        .cast_fuse()
        .args([
            "safe",
            "list-delegates",
            "0x1111111111111111111111111111111111111111",
            "--service-url",
            &service,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains("delegate pagination URL points outside the Transaction Service endpoint"),
        "unexpected error: {stderr}"
    );
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

    let stderr = cmd
        .cast_fuse()
        .args([
            "safe",
            "sign",
            &expected_arg,
            &safe_tx_hash,
            "--service-url",
            &service,
            "--rpc-url",
            &rpc,
            "--private-key",
            ANVIL_KEY,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains(&format!(
            "Transaction Service returned Safe {malicious}, expected {expected}"
        )),
        "unexpected error: {stderr}"
    );
});

casttest!(safe_simulation_requires_executor, |_prj, cmd| {
    let safe = address!("1111111111111111111111111111111111111111").to_string();
    let safe_tx_hash = B256::ZERO.to_string();

    cmd.cast_fuse();
    cmd.unset_env("ETH_FROM");
    let stderr = cmd
        .args(["safe", "simulate", &safe, &safe_tx_hash, "--rpc-url", "http://127.0.0.1:1"])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains("the following required arguments were not provided")
            && stderr.contains("--from <ADDRESS>"),
        "unexpected error: {stderr}"
    );
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
                &service,
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

    let stderr = cmd
        .cast_fuse()
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
            &service,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains("cannot simulate reimbursed Safe transactions (gasPrice > 0)"),
        "unexpected error: {stderr}"
    );
});
