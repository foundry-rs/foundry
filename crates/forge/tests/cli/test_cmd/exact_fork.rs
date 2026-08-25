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

    std::assert_eq!(rpc(&upstream, "evm_revert", json!([snapshot])).await, true);
    rpc(&upstream, "evm_setNextBlockTimestamp", json!([initial_timestamp + 30])).await;
    rpc(&upstream, "evm_mine", json!([])).await;
    rpc(&upstream, "evm_setNextBlockTimestamp", json!([initial_timestamp + 40])).await;
    rpc(&upstream, "evm_mine", json!([])).await;
    let replacement = rpc(&upstream, "eth_getBlockByNumber", json!(["latest", false])).await;
    std::assert_ne!(replacement["hash"], anchor["hash"]);
    std::assert_ne!(replacement["parentHash"], anchor["parentHash"]);

    let exact_state_read = Arc::new(AtomicBool::new(false));
    let serve_orphan_by_number = Arc::new(AtomicBool::new(true));
    let app = Router::new().fallback({
        let upstream = upstream.clone();
        let anchor = anchor.clone();
        let anchor_hash = anchor_hash.clone();
        let anchor_number = anchor_number.clone();
        let anchor_parent = anchor_parent.clone();
        let parent = parent.clone();
        let exact_state_read = Arc::clone(&exact_state_read);
        let serve_orphan_by_number = Arc::clone(&serve_orphan_by_number);
        move |body: Bytes| {
            let upstream = upstream.clone();
            let anchor = anchor.clone();
            let anchor_hash = anchor_hash.clone();
            let anchor_number = anchor_number.clone();
            let anchor_parent = anchor_parent.clone();
            let parent = parent.clone();
            let exact_state_read = Arc::clone(&exact_state_read);
            let serve_orphan_by_number = Arc::clone(&serve_orphan_by_number);
            async move {
                let mut request: Value = serde_json::from_slice(&body).unwrap();
                let id = request["id"].clone();
                let method = request["method"].as_str().unwrap();
                if method == "eth_getBlockByNumber" && request["params"][0] == "latest" {
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": anchor}));
                }
                if method == "eth_getBlockByNumber"
                    && request["params"][0] == anchor_number
                    && serve_orphan_by_number.load(Ordering::Relaxed)
                {
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": anchor}));
                }
                if method == "eth_getBlockByHash" && request["params"][0] == anchor_hash {
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": anchor}));
                }
                if method == "eth_getBlockByHash" && request["params"][0] == anchor_parent {
                    return Json(json!({"jsonrpc": "2.0", "id": id, "result": parent}));
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

    let anchor_number_value =
        u64::from_str_radix(anchor_number.trim_start_matches("0x"), 16).unwrap();
    let (fork_api, fork_handle) = spawn(
        NodeConfig::test()
            .with_eth_rpc_url(Some(endpoint.clone()))
            .with_fork_block_number(Some(anchor_number_value))
            .with_fork_chain_id(Some(U256::from(31337))),
    )
    .await;
    serve_orphan_by_number.store(false, Ordering::Relaxed);
    let orphan_hash = fork_api.backend.get_fork().unwrap().block_hash();
    let reset_number = fork_api.backend.get_fork().unwrap().block_number();
    let canonical =
        rpc(&upstream, "eth_getBlockByNumber", json!([format!("0x{reset_number:x}"), false])).await;
    std::assert_ne!(canonical["hash"], orphan_hash.to_string());
    fork_api.anvil_reset(Some(Default::default())).await.unwrap();
    std::assert_eq!(
        fork_api.backend.get_fork().unwrap().block_hash().to_string(),
        canonical["hash"]
    );
    drop(fork_handle);

    prj.add_test(
        "ExactFork.t.sol",
        &format!(
            r#"
contract ExactForkTest {{
    function testExactForkAncestry() public {{
        require(address(0x100).balance == 42, "wrong state");
        require(blockhash(block.number - 1) == bytes32({anchor_parent}), "wrong ancestry");
    }}
}}
"#
        ),
    );

    cmd.args(["test", "--fork-url", &endpoint, "--match-test", "testExactForkAncestry"])
        .assert_success();
    #[cfg(feature = "monad")]
    {
        let mut monad_cmd = prj.forge_command();
        monad_cmd
            .args([
                "test",
                "--network",
                "monad",
                "--fork-url",
                &endpoint,
                "--match-test",
                "testExactForkAncestry",
                "--threads",
                "1",
            ])
            .assert_success();
    }
    assert!(exact_state_read.load(Ordering::Relaxed));
});
