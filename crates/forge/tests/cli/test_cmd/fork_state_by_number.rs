//! Fork state compatibility with RPC endpoints that reject EIP-1898 block objects.

use alloy_primitives::{B256, U256, address, bytes};
use anvil::{NodeConfig, spawn};
use axum::{Json, Router, extract::Path, routing::post};
use foundry_config::{RpcEndpointUrl, RpcEndpoints};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc;

#[forgetest]
async fn fork_state_by_number_script_and_test(prj: _, cmd: _) {
    let (api, anvil) = spawn(NodeConfig::test().with_no_mining(true)).await;
    let target = address!("0000000000000000000000000000000000001234");
    api.anvil_set_balance(target, U256::from(42)).await.unwrap();
    api.anvil_set_code(target, bytes!("00")).await.unwrap();
    api.anvil_set_storage_at(target, U256::ZERO, B256::from(U256::from(7))).await.unwrap();
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
                        "error": {"code": -32601, "message": "method not found"}}));
                }
                if matches!(
                    method,
                    "eth_getBalance"
                        | "eth_getTransactionCount"
                        | "eth_getCode"
                        | "eth_getStorageAt"
                ) {
                    recorded.lock().push(request.clone());
                    if mode == "number"
                        && !request["params"].as_array().unwrap().last().unwrap().is_string()
                    {
                        return Json(json!({"jsonrpc": "2.0", "id": request["id"],
                            "error": {"code": -32602, "message": "block objects unsupported"}}));
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
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/number", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    prj.add_source(
        "Probe.sol",
        r#"
interface Vm {
    function load(address, bytes32) external view returns (bytes32);
    function startBroadcast() external;
    function stopBroadcast() external;
    function createSelectFork(string calldata, uint256) external returns (uint256);
    function createSelectFork(string calldata, bytes32) external returns (uint256);
    function rollFork(bytes32) external;
    function envBytes32(string calldata) external view returns (bytes32);
    function envString(string calldata) external view returns (string memory);
    function rollFork(uint256) external;
    function rpcUrl(string calldata) external view returns (string memory);
    function expectRevert(bytes calldata) external;
    function transact(bytes32) external;
    function transact(uint256, bytes32) external;
    function createFork(string calldata, bytes32) external returns (uint256);
    function selectFork(uint256) external;
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
    function testForkReplayRejectsNumberState() public {
        vm.expectRevert(bytes("vm.transact: transaction replay requires hash-addressed state; create a transaction-targeted fork or disable fork_state_by_number"));
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
    // The default still uses block hashes, which this relay rejects.
    cmd.args(["script", "src/Probe.sol:Probe", "--rpc-url", &endpoint, "--sender", &sender])
        .assert_failure();
    assert!(requests.lock().iter().any(|request| {
        request["method"] == "eth_getTransactionCount" && request["params"][1].is_object()
    }));
    for configured in [false, true] {
        prj.update_config(|config| config.fork_state_by_number = configured);
        for script in [false, true] {
            requests.lock().clear();
            let mut command = prj.forge_command();
            if script {
                command.args(["script", "src/Probe.sol:Probe", "--sender", &sender]);
            } else {
                command.args(["test", "--match-contract", "Probe"]);
            }
            command.args(["--rpc-url", &endpoint]);
            if configured {
                command.args(["--fork-block-number", "1"]);
            }
            if !configured {
                command.arg("--fork-state-by-number");
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
            for request in requests.iter() {
                assert_eq!(
                    request["params"].as_array().unwrap().last().unwrap(),
                    "0x1",
                    "{request}"
                );
            }
        }
    }
    // Also inherit the setting when the first fork is created by a cheatcode.
    prj.forge_command().args(["test", "--match-test", "testForkCheatcodes"]).assert_success();
    // Inline config controls cheatcode-created forks in both directions.
    let hash_endpoint = endpoint.replace("/number", "/hash");
    for (configured, inline) in [(false, true), (true, false)] {
        prj.update_config(|config| config.fork_state_by_number = configured);
        prj.add_test(
            "Inline.t.sol",
            &format!(
                r#"
import {{Probe}} from "../src/Probe.sol";
contract InlineProbe is Probe {{
    /// forge-config: default.fork_state_by_number = {inline}
    function testForkInlineConfig() public {{
        vm.createSelectFork(vm.envString("HASH_RPC_URL"), 1);
        checkState();
    }}
}}
"#
            ),
        );
        requests.lock().clear();
        let mut command = prj.forge_command();
        command.env("HASH_RPC_URL", &hash_endpoint);
        command.args(["test", "--match-test", "testForkInlineConfig"]).assert_success();
        let reads = requests.lock();
        let target_reads = reads
            .iter()
            .filter(|request| request["params"][0] == json!(target))
            .collect::<Vec<_>>();
        assert!(!target_reads.is_empty());
        for request in target_reads {
            let block = request["params"].as_array().unwrap().last().unwrap();
            assert_eq!(block.is_string(), inline, "configured={configured}, {request}");
        }
    }
    prj.update_config(|config| config.fork_state_by_number = true);
    // Numbered rolls must move between blocks with different state, not stay pinned or follow head.
    // Anvil applies state overrides to the latest block, so mine first: block 2 = 9, head = 11.
    api.mine_one().await.unwrap();
    api.anvil_set_storage_at(target, U256::from(1), B256::from(U256::from(9))).await.unwrap();
    api.mine_one().await.unwrap();
    api.anvil_set_storage_at(target, U256::from(1), B256::from(U256::from(11))).await.unwrap();
    prj.add_test(
        "Roll.t.sol",
        r#"
import {Probe} from "../src/Probe.sol";
contract RollProbe is Probe {
    function slot1() internal view returns (uint256) {
        return uint256(vm.load(address(0x1234), bytes32(uint256(1))));
    }
    function testForkRollCrossesBlocks() public {
        vm.createSelectFork(vm.rpcUrl("relay"), 1);
        checkState();
        require(slot1() == 0, "pinned block");
        vm.rollFork(2);
        checkState();
        require(slot1() == 9, "rolled forward");
        vm.rollFork(1);
        require(slot1() == 0, "rolled back");
    }
}
"#,
    );
    requests.lock().clear();
    prj.forge_command()
        .args(["test", "--match-test", "testForkRollCrossesBlocks"])
        .assert_success();
    let blocks = requests
        .lock()
        .iter()
        .filter(|request| request["method"] == "eth_getStorageAt")
        .map(|request| request["params"].as_array().unwrap().last().unwrap().clone())
        .collect::<Vec<_>>();
    assert!(blocks.contains(&json!("0x1")) && blocks.contains(&json!("0x2")), "{blocks:?}");
    assert!(blocks.iter().all(|block| block == "0x1" || block == "0x2"), "{blocks:?}");
    // Transaction-targeted creation and rolling still use hashes even with the opt-in enabled.
    let transaction = api
        .send_transaction(
            serde_json::from_value(json!({
                "from": sender, "to": target, "gas": "0x186a0", "gasPrice": "0x77359400"
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    prj.add_test(
        "Replay.t.sol",
        r#"
import {Probe} from "../src/Probe.sol";
contract ReplayProbe is Probe {
    function testForkTransactionTargetsUseHashes() public {
        string memory url = vm.envString("HASH_RPC_URL");
        bytes32 transaction = vm.envBytes32("TARGET_TRANSACTION");
        vm.createSelectFork(url, transaction);
        checkState();
        vm.transact(transaction);
        vm.createSelectFork(url, 1);
        vm.rollFork(transaction);
        checkState();
    }
    function testForkTransactTargetsExplicitFork() public {
        bytes32 transaction = vm.envBytes32("TARGET_TRANSACTION");
        uint256 hashFork = vm.createFork(vm.envString("HASH_RPC_URL"), transaction);
        uint256 numberFork = vm.createSelectFork(vm.rpcUrl("relay"), 1);
        // The explicit hash-addressed target is accepted while a number-addressed fork is active.
        vm.transact(hashFork, transaction);
        vm.selectFork(hashFork);
        vm.expectRevert(bytes("vm.transact: transaction replay requires hash-addressed state; create a transaction-targeted fork or disable fork_state_by_number"));
        this.replayOn(numberFork, transaction);
    }
    function replayOn(uint256 forkId, bytes32 transaction) external {
        vm.transact(forkId, transaction);
    }
}
"#,
    );
    requests.lock().clear();
    let mut command = prj.forge_command();
    command.env("HASH_RPC_URL", &hash_endpoint);
    command.env("TARGET_TRANSACTION", transaction.to_string());
    command.args(["test", "--match-test", "testForkTransactionTargetsUseHashes"]).assert_success();
    let reads = requests.lock();
    let target_reads =
        reads.iter().filter(|request| request["params"][0] == json!(target)).collect::<Vec<_>>();
    assert!(!target_reads.is_empty());
    assert!(
        target_reads.iter().all(|request| request["params"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .is_object())
    );
    drop(reads);
    let mut command = prj.forge_command();
    command.env("HASH_RPC_URL", &hash_endpoint);
    command.env("TARGET_TRANSACTION", transaction.to_string());
    command.args(["test", "--match-test", "testForkTransactTargetsExplicitFork"]).assert_success();
    server.abort();
}
