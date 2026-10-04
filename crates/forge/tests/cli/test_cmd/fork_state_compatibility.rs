//! Fork state compatibility with RPC endpoints that reject EIP-1898 block objects.

use alloy_primitives::{B256, U256, address, bytes};
use alloy_provider::Provider;
use anvil::{NodeConfig, spawn};
use axum::{Json, Router, extract::Path, http::StatusCode, response::IntoResponse, routing::post};
use foundry_config::{RpcEndpointUrl, RpcEndpoints};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;

#[forgetest]
async fn fork_state_compatibility_script_and_test(prj: _) {
    let (api, anvil) = spawn(NodeConfig::test().with_no_mining(true)).await;
    let target = address!("0000000000000000000000000000000000001234");
    api.anvil_set_balance(target, U256::from(42)).await.unwrap();
    api.anvil_set_code(target, bytes!("00")).await.unwrap();
    api.anvil_set_storage_at(target, U256::ZERO, B256::from(U256::from(7))).await.unwrap();
    api.mine_one().await.unwrap();
    let old_hash =
        anvil.http_provider().get_block_by_number(1.into()).await.unwrap().unwrap().header.hash;
    api.mine_one().await.unwrap();
    let sender = anvil.dev_wallets().next().unwrap().address().to_string();
    let upstream = anvil.http_endpoint();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let app = Router::new().route(
        "/{mode}",
        post(move |Path(mode): Path<String>, Json(request): Json<Value>| {
            let upstream = upstream.clone();
            let recorded = Arc::clone(&recorded);
            async move {
                let method = request["method"].as_str().unwrap();
                // Exercise the path used by public RPC providers rather than Anvil discovery.
                if matches!(method, "anvil_nodeInfo" | "anvil_metadata") {
                    return Json(json!({"jsonrpc": "2.0", "id": request["id"],
                        "error": {"code": -32601, "message": "method not found"}}))
                    .into_response();
                }
                if matches!(
                    method,
                    "eth_getBalance"
                        | "eth_getTransactionCount"
                        | "eth_getCode"
                        | "eth_getStorageAt"
                ) {
                    recorded.lock().push(request.clone());
                    let selector = request["params"].as_array().unwrap().last().unwrap();
                    if selector.is_object()
                        && (mode == "relay" || selector["blockHash"] == json!(old_hash))
                    {
                        // Hedera rejects objects over HTTP 400. Monad accepts them for recent
                        // blocks, but returns invalid params for older historical state.
                        let (status, message) = if mode == "relay" {
                            (StatusCode::BAD_REQUEST, "Invalid parameter: [object Object]")
                        } else {
                            (StatusCode::OK, "Block requested not found")
                        };
                        return (
                            status,
                            Json(json!({"jsonrpc": "2.0", "id": if mode == "relay" { Value::Null } else { request["id"].clone() },
                            "error": {"code": -32602, "message": message}})),
                        )
                            .into_response();
                    }
                }
                Json(
                    reqwest::Client::new()
                        .post(upstream)
                        .json(&request)
                        .send()
                        .await
                        .unwrap()
                        .json::<Value>()
                        .await
                        .unwrap(),
                )
                .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/relay", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    prj.add_source(
        "Probe.sol",
        r#"
interface Vm {
    function load(address, bytes32) external view returns (bytes32);
    function startBroadcast() external;
    function stopBroadcast() external;
    function createSelectFork(string calldata, uint256) external returns (uint256);
    function rollFork(uint256) external;
    function rpcUrl(string calldata) external view returns (string memory);
    function expectRevert(bytes calldata) external;
    function transact(bytes32) external;
}
contract Counter {
    uint256 public n;
    function inc() external { n++; }
}
contract Probe {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    function checkState() internal view {
        require(address(0x1234).balance == 42, "balance");
        require(address(0x1234).code.length == 1, "code");
        require(uint256(vm.load(address(0x1234), bytes32(0))) == 7, "storage");
    }
    function testForkState() public view { checkState(); }
    function testForkCheatcodes() public {
        vm.createSelectFork(vm.rpcUrl("relay"), 1);
        checkState();
        vm.rollFork(1);
        checkState();
    }
    function testReplayRejectsNumberState() public {
        checkState();
        vm.expectRevert(bytes("vm.transact: transaction replay requires hash-addressed state; create a fresh transaction-targeted fork"));
        this.replay();
    }
    function replay() external { vm.transact(bytes32(0)); }
    function run() public {
        checkState();
        vm.startBroadcast();
        new Counter().inc();
        vm.stopBroadcast();
    }
}
"#,
    );
    prj.update_config(|config| {
        config.no_storage_caching = true;
        config.rpc_endpoints =
            RpcEndpoints::new([("relay", RpcEndpointUrl::Url(endpoint.clone()))]);
    });
    for explicit_block in [false, true] {
        for script in [false, true] {
            requests.lock().clear();
            let mut command = prj.forge_command();
            if script {
                command.args(["script", "src/Probe.sol:Probe", "--sender", &sender]);
            } else {
                command.args(["test", "--match-contract", "Probe"]);
            }
            command.args(["--rpc-url", &endpoint]);
            if explicit_block {
                command.args(["--fork-block-number", "1"]);
            }
            command.assert_success();
            let requests = requests.lock();
            for method in
                ["eth_getBalance", "eth_getTransactionCount", "eth_getCode", "eth_getStorageAt"]
            {
                assert!(
                    requests.iter().any(|request| request["method"] == method),
                    "missing {method}"
                );
            }
            assert!(
                requests.iter().any(|request| request["params"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()
                    .is_object())
            );
            assert!(
                requests.iter().any(|request| request["params"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()
                    .is_string())
            );
            for request in requests.iter() {
                let selector = request["params"].as_array().unwrap().last().unwrap();
                if selector.is_object() {
                    assert_eq!(selector.as_object().unwrap().len(), 1);
                    assert!(selector["blockHash"].is_string());
                } else {
                    assert!(selector == "0x1" || selector == "0x2", "{selector}");
                }
            }
        }
    }
    // The same policy applies when the first fork is created by a cheatcode.
    prj.forge_command().args(["test", "--match-test", "testForkCheatcodes"]).assert_success();
    let archive = endpoint.replace("/relay", "/archive");
    for block in ["1", "2"] {
        requests.lock().clear();
        prj.forge_command()
            .args([
                "test",
                "--match-test",
                "testForkState",
                "--rpc-url",
                &archive,
                "--fork-block-number",
                block,
            ])
            .assert_success();
        let requests = requests.lock();
        assert!(
            requests.iter().any(|request| request["params"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .is_object())
        );
        assert_eq!(
            requests.iter().any(|request| request["params"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .is_string()),
            block == "1"
        );
    }
    server.abort();
}
