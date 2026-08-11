use super::*;
use axum::{Json, Router, body::Bytes};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

async fn rpc(endpoint: &str, method: &str, params: Value) -> Value {
    reqwest::Client::new()
        .post(endpoint)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["result"]
        .clone()
}

forgetest_async!(fork_execution_uses_exact_ancestry_after_reorg, |prj, cmd| {
    let (_api, anvil) = spawn(NodeConfig::test()).await;
    let upstream = anvil.http_endpoint();
    let initial = rpc(&upstream, "eth_getBlockByNumber", json!(["latest", false])).await;
    let initial_timestamp =
        u64::from_str_radix(initial["timestamp"].as_str().unwrap().trim_start_matches("0x"), 16)
            .unwrap();
    let snapshot = rpc(&upstream, "evm_snapshot", json!([])).await;

    rpc(&upstream, "evm_setNextBlockTimestamp", json!([initial_timestamp + 10])).await;
    rpc(&upstream, "evm_mine", json!([])).await;
    rpc(&upstream, "evm_setNextBlockTimestamp", json!([initial_timestamp + 20])).await;
    rpc(&upstream, "evm_mine", json!([])).await;
    let anchor = rpc(&upstream, "eth_getBlockByNumber", json!(["latest", false])).await;
    let anchor_hash = anchor["hash"].as_str().unwrap().to_string();
    let anchor_number = anchor["number"].as_str().unwrap().to_string();
    let anchor_parent = anchor["parentHash"].as_str().unwrap().to_string();
    let parent = rpc(&upstream, "eth_getBlockByHash", json!([anchor_parent, false])).await;
    let parent_number = parent["number"].as_str().unwrap().to_string();
    let parent_number_value =
        u64::from_str_radix(parent_number.trim_start_matches("0x"), 16).unwrap();

    std::assert_eq!(rpc(&upstream, "evm_revert", json!([snapshot])).await, true);
    rpc(&upstream, "evm_setNextBlockTimestamp", json!([initial_timestamp + 30])).await;
    rpc(&upstream, "evm_mine", json!([])).await;
    rpc(&upstream, "evm_setNextBlockTimestamp", json!([initial_timestamp + 40])).await;
    rpc(&upstream, "evm_mine", json!([])).await;
    let replacement = rpc(&upstream, "eth_getBlockByNumber", json!(["latest", false])).await;
    std::assert_ne!(replacement["hash"], anchor["hash"]);
    std::assert_ne!(replacement["parentHash"], anchor["parentHash"]);

    let exact_state_read = Arc::new(AtomicBool::new(false));
    let app = Router::new().fallback({
        let upstream = upstream.clone();
        let anchor = anchor.clone();
        let anchor_hash = anchor_hash.clone();
        let anchor_number = anchor_number.clone();
        let anchor_parent = anchor_parent.clone();
        let parent = parent.clone();
        let parent_number = parent_number.clone();
        let exact_state_read = Arc::clone(&exact_state_read);
        move |body: Bytes| {
            let upstream = upstream.clone();
            let anchor = anchor.clone();
            let anchor_hash = anchor_hash.clone();
            let anchor_number = anchor_number.clone();
            let anchor_parent = anchor_parent.clone();
            let parent = parent.clone();
            let parent_number = parent_number.clone();
            let exact_state_read = Arc::clone(&exact_state_read);
            async move {
                let mut request: Value = serde_json::from_slice(&body).unwrap();
                let id = request["id"].clone();
                let method = request["method"].as_str().unwrap();
                if method == "eth_getBlockByNumber" && request["params"][0] == "latest" {
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": anchor}));
                }
                if method == "eth_getBlockByNumber" && request["params"][0] == parent_number {
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": parent}));
                }
                if method == "eth_getBlockByHash" && request["params"][0] == anchor_hash {
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": anchor}));
                }
                let state_block_hash = if matches!(
                    method,
                    "eth_getBalance"
                        | "eth_getTransactionCount"
                        | "eth_getCode"
                        | "eth_getStorageAt"
                ) {
                    request["params"]
                        .as_array()
                        .and_then(|params| params.last())
                        .and_then(|block| block.get("blockHash"))
                        .and_then(Value::as_str)
                } else {
                    None
                };
                let exact_state = state_block_hash == Some(&anchor_hash);
                let parent_state = state_block_hash == Some(&anchor_parent);
                if exact_state
                    && method == "eth_getBalance"
                    && request["params"][0] == "0x0000000000000000000000000000000000000100"
                {
                    exact_state_read.store(true, Ordering::Relaxed);
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": "0x2a"}));
                }
                if exact_state {
                    let params = request["params"].as_array_mut().unwrap();
                    *params.last_mut().unwrap() = Value::String(anchor_number);
                } else if parent_state {
                    let params = request["params"].as_array_mut().unwrap();
                    *params.last_mut().unwrap() = Value::String(parent_number);
                }
                let response = reqwest::Client::new()
                    .post(upstream)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .json::<Value>()
                    .await
                    .unwrap();
                Json(response)
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    prj.add_test(
        "ExactFork.t.sol",
        &format!(
            r#"
interface Vm {{
    function rollFork(uint256 blockNumber) external;
}}

contract ExactForkTest {{
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testExactForkAncestry() public {{
        require(address(0x100).balance == 42, "wrong state");
        require(blockhash(block.number - 1) == bytes32({anchor_parent}), "wrong ancestry");
        vm.rollFork({parent_number_value});
        require(block.number == {parent_number_value}, "wrong rolled block");
    }}
}}
"#
        ),
    );

    cmd.args(["test", "--fork-url", &endpoint, "--match-test", "testExactForkAncestry"])
        .assert_success();
    assert!(exact_state_read.load(Ordering::Relaxed));
});
