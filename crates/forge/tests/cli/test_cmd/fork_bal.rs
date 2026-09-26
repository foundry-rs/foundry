//! Transaction-hash fork cache warming against a locally mined parent and transaction prefix.

use alloy_primitives::{Address, B256, U256, address, bytes};
use anvil::{EthereumHardfork, NodeConfig, NodeHandle, spawn};
use axum::{Json, Router, routing::post};
use foundry_test_utils::TestCommand;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;

const COUNTER: Address = address!("000000000000000000000000000000000000ba10");
const BAL_METHOD: &str = "eth_getBlockAccessList";

async fn rpc(endpoint: &str, method: &str, params: Value) -> Value {
    let response = reqwest::Client::new()
        .post(endpoint)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(response.get("error").is_none(), "{method}: {response}");
    response["result"].clone()
}

struct Fixture {
    handle: NodeHandle,
    parent: Value,
    transactions: [B256; 3],
    pending: B256,
}

impl Fixture {
    async fn new() -> Self {
        // Amsterdam makes Anvil produce native BALs and their block-header commitments.
        let (api, handle) = spawn(
            NodeConfig::test()
                .with_chain_id(Some(1u64))
                .with_hardfork(Some(EthereumHardfork::Amsterdam.into()))
                .with_genesis_timestamp(Some(1_800_000_000u64))
                .with_no_mining(true),
        )
        .await;
        // Every transaction reads slot one and increments slot zero.
        api.anvil_set_code(COUNTER, bytes!("6001545060005460010160005500")).await.unwrap();
        api.anvil_set_storage_at(COUNTER, U256::ZERO, B256::from(U256::from(6))).await.unwrap();
        api.anvil_set_storage_at(COUNTER, U256::from(1), B256::from(U256::from(19))).await.unwrap();
        let endpoint = handle.http_endpoint();
        let sender = handle.dev_wallets().next().unwrap().address();
        let send = |nonce| {
            json!([{"from": sender, "to": COUNTER, "nonce": format!("0x{nonce:x}"),
                    "gas": "0x30d40", "gasPrice": "0x77359400"}])
        };
        rpc(&endpoint, "eth_sendTransaction", send(0)).await;
        api.mine_one().await.unwrap();
        let parent = rpc(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
        assert!(parent["blockAccessListHash"].is_string(), "missing native BAL commitment");
        assert_eq!(
            rpc(&endpoint, "eth_getStorageAt", json!([COUNTER, "0x0", "latest"])).await,
            json!(B256::from(U256::from(7))),
        );
        let mut transactions = [B256::ZERO; 3];
        for (index, hash) in transactions.iter_mut().enumerate() {
            *hash = serde_json::from_value(
                rpc(&endpoint, "eth_sendTransaction", send(index + 1)).await,
            )
            .unwrap();
        }
        api.mine_one().await.unwrap();
        let block = rpc(&endpoint, "eth_getBlockByNumber", json!(["latest", false])).await;
        assert_eq!(block["transactions"], json!(transactions));
        assert_eq!(block["parentHash"], parent["hash"]);
        let pending =
            serde_json::from_value(rpc(&endpoint, "eth_sendTransaction", send(4)).await).unwrap();
        Self { handle, parent, transactions, pending }
    }
}

#[derive(Clone, Copy, Debug)]
enum Response {
    Native,
    Unsupported,
    InvalidCode,
    Timeout,
    Anvil,
    CumulativeTimeout,
    InvalidPrefix,
    NullOnce,
}

struct Proxy {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl Proxy {
    async fn new(fixture: &Fixture, mode: Response) -> Self {
        let upstream = fixture.handle.http_endpoint();
        let client = reqwest::Client::new();
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recorded = Arc::clone(&requests);
        let parent_hash = fixture.parent["hash"].clone();
        let app = Router::new().route(
            "/",
            post(move |Json(request): Json<Value>| {
                let upstream = upstream.clone();
                let client = client.clone();
                let recorded = Arc::clone(&recorded);
                let parent_hash = parent_hash.clone();
                async move {
                    let after_bal = {
                        let mut requests = recorded.lock();
                        let after_bal =
                            requests.iter().any(|request| request["method"] == BAL_METHOD);
                        requests.push(request.clone());
                        after_bal
                    };
                    let method = request["method"].as_str().unwrap();
                    if matches!(method, "anvil_nodeInfo" | "anvil_metadata") {
                        if matches!(mode, Response::CumulativeTimeout) && after_bal {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                        // Hide the local-node identity while leaving native block data intact.
                        if !matches!(mode, Response::Anvil) {
                            return Json(json!({"jsonrpc": "2.0", "id": request["id"],
                                "error": {"code": -32601, "message": "method not found"}}));
                        }
                    }
                    let bal_method = method == BAL_METHOD;
                    if bal_method && matches!(mode, Response::Unsupported) {
                        return Json(json!({"jsonrpc": "2.0", "id": request["id"],
                            "error": {"code": -32601, "message": "method not found"}}));
                    }
                    if bal_method {
                        if matches!(mode, Response::CumulativeTimeout) {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                        if matches!(mode, Response::Timeout) {
                            return futures::future::pending().await;
                        }
                        if matches!(mode, Response::NullOnce) && !after_bal {
                            return Json(
                                json!({"jsonrpc": "2.0", "id": request["id"], "result": null}),
                            );
                        }
                        if matches!(mode, Response::InvalidCode) {
                            // Valid storage must not enter the cache if a later account is invalid.
                            let bal = json!([{
                                "address": COUNTER,
                                "storageChanges": [{"key": "0x0", "changes": [
                                    {"index": "0x1", "value": "0x7"}
                                ]}],
                                "storageReads": ["0x1"],
                                "balanceChanges": [], "nonceChanges": [], "codeChanges": []
                            }, {
                                "address": "0x000000000000000000000000000000000000ba11",
                                "storageChanges": [], "storageReads": [],
                                "balanceChanges": [], "nonceChanges": [],
                                "codeChanges": [
                                    {"index": "0x0", "code": "0xef0100"},
                                    {"index": "0x1", "code": "0x"}
                                ]
                            }]);
                            return Json(
                                json!({"jsonrpc": "2.0", "id": request["id"], "result": bal}),
                            );
                        }
                    }
                    let mut response = client
                        .post(upstream)
                        .json(&request)
                        .send()
                        .await
                        .unwrap()
                        .json::<Value>()
                        .await
                        .unwrap();
                    if matches!(method, "eth_getBlockByHash" | "eth_getBlockByNumber") {
                        // Exercise bytecode validation without an earlier commitment mismatch.
                        if matches!(mode, Response::InvalidCode) {
                            response["result"]
                                .as_object_mut()
                                .unwrap()
                                .remove("blockAccessListHash");
                        }
                        if matches!(mode, Response::InvalidPrefix)
                            && response["result"]["parentHash"] == parent_hash
                            && request["params"][1] == true
                        {
                            response["result"]["transactions"][0]["gas"] = json!("0x0");
                        }
                    }
                    Json(response)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { endpoint, requests, task }
    }

    fn count(&self, method: &str) -> usize {
        self.requests.lock().iter().filter(|request| request["method"] == method).count()
    }

    fn slot_reads(&self, slot: U256) -> usize {
        self.requests
            .lock()
            .iter()
            .filter(|request| {
                request["method"] == "eth_getStorageAt"
                    && request["params"][0] == json!(COUNTER)
                    && serde_json::from_value::<U256>(request["params"][1].clone()).unwrap() == slot
            })
            .count()
    }

    fn assert_parent_bal(&self, fixture: &Fixture) {
        let requests = self.requests.lock();
        let calls =
            requests.iter().filter(|request| request["method"] == BAL_METHOD).collect::<Vec<_>>();
        assert!(!calls.is_empty(), "parent BAL was never requested");
        for request in calls {
            assert_eq!(request["params"], json!([fixture.parent["hash"]]));
        }
    }

    fn clear(&self) {
        self.requests.lock().clear();
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

const TEST: &str = r#"
interface Vm {
    function envString(string calldata) external returns (string memory);
    function envUint(string calldata) external returns (uint256);
    function envBytes32(string calldata) external returns (bytes32);
    function createFork(string calldata, uint256) external returns (uint256);
    function createFork(string calldata, bytes32) external returns (uint256);
    function createSelectFork(string calldata, uint256) external returns (uint256);
    function createSelectFork(string calldata, bytes32) external returns (uint256);
    function selectFork(uint256) external;
    function activeFork() external view returns (uint256);
    function rollFork(bytes32) external;
    function rollFork(uint256, bytes32) external;
    function load(address, bytes32) external view returns (bytes32);
    function store(address, bytes32, bytes32) external;
    function makePersistent(address) external;
    function snapshotState() external returns (uint256);
    function revertToState(uint256) external returns (bool);
    function _expectCheatcodeRevert() external;
}

contract ForkBalTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant counter = address(0xba10);

    function value() internal view returns (uint256) {
        return uint256(vm.load(counter, bytes32(0)));
    }

    function testForkBal() public {
        string memory url = vm.envString("BAL_RPC_URL");
        bytes32 target = vm.envBytes32("BAL_TARGET");
        uint256 parent = vm.envUint("BAL_PARENT");
        uint256 mode = vm.envUint("BAL_MODE");
        if (mode == 0) {
            vm.selectFork(vm.createFork(url, target));
        } else if (mode == 1) {
            vm.createSelectFork(url, target);
        } else if (mode == 2) {
            vm.createSelectFork(url, parent);
            vm.store(address(0xba11), bytes32(0), bytes32(uint256(70)));
            vm.rollFork(target);
            require(vm.load(address(0xba11), bytes32(0)) == bytes32(0), "active roll retained local state");
        } else {
            uint256 active = vm.createSelectFork(url, parent);
            uint256 inactive = vm.createFork(url, parent);
            vm.rollFork(inactive, target);
            require(vm.activeFork() == active, "inactive roll selected the fork");
            require(value() == 7, "inactive roll changed active state");
            vm.selectFork(inactive);
        }
        require(value() == vm.envUint("BAL_EXPECTED"), "wrong prefix state");
        require(uint256(vm.load(counter, bytes32(uint256(1)))) == 19, "lost read-only slot");
    }

    function testForkBalLifecycle() public {
        string memory url = vm.envString("BAL_RPC_URL");
        bytes32 target = vm.envBytes32("BAL_TARGET");
        uint256 ordinary = vm.createFork(url, vm.envUint("BAL_PARENT"));
        uint256 first = vm.createSelectFork(url, target);
        require(value() == 9, "prefix missing");
        uint256 snapshot = vm.snapshotState();
        (bool ok,) = counter.call("");
        require(ok && value() == 10, "local transaction missing");
        require(vm.revertToState(snapshot) && value() == 9, "snapshot lost");
        vm.store(counter, bytes32(0), bytes32(uint256(90)));
        address persistent = address(0xba11);
        vm.store(persistent, bytes32(0), bytes32(uint256(42)));
        vm.makePersistent(persistent);
        uint256 second = vm.createSelectFork(url, target);
        require(value() == 9, "new fork inherited local write");
        require(uint256(vm.load(persistent, bytes32(0))) == 42, "persistent state lost");
        vm.store(counter, bytes32(0), bytes32(uint256(80)));
        vm.selectFork(first);
        require(value() == 90, "first fork local write lost");
        vm.selectFork(second);
        require(value() == 80, "second fork local write lost");
        vm.selectFork(ordinary);
        require(value() == 7, "ordinary fork inherited prefix state");
    }

    function testForkBalRejectsInvalidPrefix() public {
        string memory url = vm.envString("BAL_RPC_URL");
        bytes32 target = vm.envBytes32("BAL_TARGET");
        vm._expectCheatcodeRevert();
        vm.createSelectFork(url, target);
    }

    function testForkBalOrdinary() public {
        vm.createSelectFork(vm.envString("BAL_RPC_URL"), vm.envUint("BAL_PARENT"));
        require(value() == 7, "wrong ordinary fork");
    }

    function testForkBalRepeated() public {
        string memory url = vm.envString("BAL_RPC_URL");
        bytes32 target = vm.envBytes32("BAL_TARGET");
        for (uint256 i; i < 3; ++i) {
            vm.createSelectFork(url, target);
            require(value() == 9, "wrong repeated prefix state");
            vm.store(counter, bytes32(0), bytes32(uint256(90)));
        }
    }

    function testForkBalSeparateSources() public {
        string memory first = vm.envString("BAL_RPC_URL");
        string memory second = vm.envString("BAL_OTHER_RPC_URL");
        bytes32 target = vm.envBytes32("BAL_TARGET");
        for (uint256 i; i < 2; ++i) {
            vm.createSelectFork(first, target);
            require(value() == 9, "wrong first source prefix state");
            vm.createSelectFork(second, target);
            require(value() == 9, "wrong second source prefix state");
        }
    }
}
"#;

fn command<'a>(
    cmd: &'a mut TestCommand,
    fixture: &Fixture,
    proxy: &Proxy,
    target: B256,
    expected: u64,
    mode: u64,
    test: &str,
) -> &'a mut TestCommand {
    let parent = u64::from_str_radix(
        fixture.parent["number"].as_str().unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap();
    cmd.forge_fuse();
    cmd.cmd().env_remove("FOUNDRY_NO_FORK_BAL");
    cmd.env("BAL_RPC_URL", &proxy.endpoint);
    cmd.env("BAL_TARGET", target.to_string());
    cmd.env("BAL_PARENT", parent.to_string());
    cmd.env("BAL_MODE", mode.to_string());
    cmd.env("BAL_EXPECTED", expected.to_string());
    cmd.env("FOUNDRY_NO_STORAGE_CACHING", "true");
    cmd.env("FOUNDRY_DISABLE_NIGHTLY_WARNING", "true");
    cmd.args(["test", "--match-test", test, "--evm-version", "cancun"])
}

fn assert_test(cmd: &mut TestCommand, name: &str) -> u64 {
    let output = cmd.assert_success().stdout_eq(format!(
        "...\nRan 1 test for test/ForkBal.t.sol:ForkBalTest\n[PASS] {name}() ([GAS])\nSuite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]\n\nRan 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)\n",
    ));
    let stdout = std::str::from_utf8(&output.get_output().stdout).unwrap();
    let prefix = format!("[PASS] {name}() (gas: ");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix).and_then(|gas| gas.strip_suffix(')')))
        .expect("unit test gas is reported")
        .parse()
        .unwrap()
}

forgetest_async!(fork_bal_parent_cache_preserves_prefix_boundaries, |prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = Proxy::new(&fixture, Response::Native).await;
    prj.add_test("ForkBal.t.sol", TEST);
    // Mode 1 covers empty and multi-transaction prefixes. The other modes exercise their fork
    // lifecycle with the final transaction, which is also the path used by the other tests.
    for (mode, index) in [(1, 0), (1, 2), (0, 2), (2, 2), (3, 2)] {
        let mut block_reads = Vec::new();
        let mut gas_used = Vec::new();
        for disabled in [false, true] {
            proxy.clear();
            command(
                &mut cmd,
                &fixture,
                &proxy,
                fixture.transactions[index],
                7 + index as u64,
                mode,
                r"^testForkBal\(\)$",
            );
            if disabled {
                cmd.arg("--no-fork-bal");
            }
            gas_used.push(assert_test(&mut cmd, "testForkBal"));
            block_reads.push(proxy.count("eth_getBlockByHash"));
            if disabled {
                assert_eq!(proxy.count(BAL_METHOD), 0);
                assert!(proxy.slot_reads(U256::ZERO) > 0);
            } else {
                proxy.assert_parent_bal(&fixture);
                assert_eq!(proxy.slot_reads(U256::ZERO), 0, "mode={mode}, index={index}");
            }
            assert!(proxy.slot_reads(U256::from(1)) > 0, "read-only slots need RPC fallback");
        }
        assert_eq!(block_reads[0], block_reads[1], "BAL fetched an extra block: mode={mode}");
        assert_eq!(gas_used[0], gas_used[1], "BAL changed gas: mode={mode}, index={index}");
    }
});

forgetest_async!(fork_bal_keeps_local_writes_snapshots_and_persistent_accounts, |prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = Proxy::new(&fixture, Response::Native).await;
    prj.add_test("ForkBal.t.sol", TEST);
    let mut gas_used = Vec::new();
    let mut probes = Vec::new();
    for disabled in [false, true] {
        proxy.clear();
        command(
            &mut cmd,
            &fixture,
            &proxy,
            fixture.transactions[2],
            9,
            1,
            r"^testForkBalLifecycle\(\)$",
        );
        if disabled {
            cmd.arg("--no-fork-bal");
        }
        gas_used.push(assert_test(&mut cmd, "testForkBalLifecycle"));
        probes.push(proxy.count("anvil_nodeInfo"));
        if disabled {
            assert_eq!(proxy.count(BAL_METHOD), 0);
        } else {
            proxy.assert_parent_bal(&fixture);
            assert_eq!(proxy.count(BAL_METHOD), 1, "the same parent cache was prewarmed twice");
            assert_eq!(proxy.slot_reads(U256::ZERO), 0);
        }
    }
    assert_eq!(gas_used[0], gas_used[1], "BAL changed local execution gas");
    // Only the first seed adds source probes; ordinary identity checks remain on reuse.
    assert!(probes[1] > 0);
    assert_eq!(probes[0], probes[1] + 2);
});

forgetest_async!(fork_bal_config_and_environment_control_runtime_requests, |prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = Proxy::new(&fixture, Response::Native).await;
    prj.add_test("ForkBal.t.sol", TEST);
    prj.update_config(|config| config.no_fork_bal = true);
    for (environment, enabled) in [(None, false), (Some("false"), true)] {
        proxy.clear();
        command(&mut cmd, &fixture, &proxy, fixture.transactions[2], 9, 1, r"^testForkBal\(\)$");
        if let Some(environment) = environment {
            cmd.env("FOUNDRY_NO_FORK_BAL", environment);
        }
        assert_test(&mut cmd, "testForkBal");
        if enabled {
            proxy.assert_parent_bal(&fixture);
            assert_eq!(proxy.slot_reads(U256::ZERO), 0);
        } else {
            assert_eq!(proxy.count(BAL_METHOD), 0);
            assert!(proxy.slot_reads(U256::ZERO) > 0);
        }
    }
});

forgetest_async!(fork_bal_unusable_responses_fall_back_to_replay, |prj, cmd| {
    let fixture = Fixture::new().await;
    prj.add_test("ForkBal.t.sol", TEST);
    let baseline = Proxy::new(&fixture, Response::Native).await;
    command(&mut cmd, &fixture, &baseline, fixture.transactions[2], 9, 1, r"^testForkBal\(\)$")
        .arg("--no-fork-bal");
    let gas_used = assert_test(&mut cmd, "testForkBal");
    let block_reads = baseline.count("eth_getBlockByHash");
    assert_eq!(baseline.count(BAL_METHOD), 0);
    assert!(baseline.slot_reads(U256::ZERO) > 0);
    // Core BAL tests cover the remaining validation and source-eligibility combinations.
    for mode in [
        Response::Unsupported,
        Response::InvalidCode,
        Response::Timeout,
        Response::CumulativeTimeout,
    ] {
        let proxy = Proxy::new(&fixture, mode).await;
        command(&mut cmd, &fixture, &proxy, fixture.transactions[2], 9, 1, r"^testForkBal\(\)$");
        let started = Instant::now();
        assert_eq!(assert_test(&mut cmd, "testForkBal"), gas_used, "BAL changed gas: {mode:?}");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "optional BAL request delayed the fork: {mode:?}"
        );
        proxy.assert_parent_bal(&fixture);
        assert!(proxy.slot_reads(U256::ZERO) > 0, "invalid BAL was used: {mode:?}");
        assert_eq!(proxy.count("eth_getBlockByHash"), block_reads, "extra block read: {mode:?}");
    }
});

forgetest_async!(fork_bal_skips_ineligible_ordinary_and_pending_forks, |prj, cmd| {
    let fixture = Fixture::new().await;
    prj.add_test("ForkBal.t.sol", TEST);
    let proxy = Proxy::new(&fixture, Response::Anvil).await;
    command(&mut cmd, &fixture, &proxy, fixture.transactions[0], 7, 1, r"^testForkBal\(\)$");
    assert_test(&mut cmd, "testForkBal");
    assert_eq!(proxy.count(BAL_METHOD), 0, "mutable Anvil source");
    let proxy = Proxy::new(&fixture, Response::Native).await;
    command(
        &mut cmd,
        &fixture,
        &proxy,
        fixture.transactions[2],
        7,
        1,
        r"^testForkBalOrdinary\(\)$",
    );
    assert_test(&mut cmd, "testForkBalOrdinary");
    assert_eq!(proxy.count(BAL_METHOD), 0);
    proxy.clear();
    command(&mut cmd, &fixture, &proxy, fixture.pending, 10, 1, r"^testForkBal\(\)$");
    assert_test(&mut cmd, "testForkBal");
    assert_eq!(proxy.count(BAL_METHOD), 0);
});

forgetest_async!(fork_bal_preserves_prefix_transaction_validation, |prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = Proxy::new(&fixture, Response::InvalidPrefix).await;
    prj.add_test("ForkBal.t.sol", TEST);
    command(
        &mut cmd,
        &fixture,
        &proxy,
        fixture.transactions[2],
        9,
        1,
        r"^testForkBalRejectsInvalidPrefix\(\)$",
    );
    assert_test(&mut cmd, "testForkBalRejectsInvalidPrefix");
    proxy.assert_parent_bal(&fixture);
});

forgetest_async!(fork_bal_retries_unavailable_parent_seed, |prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = Proxy::new(&fixture, Response::NullOnce).await;
    prj.add_test("ForkBal.t.sol", TEST);
    command(
        &mut cmd,
        &fixture,
        &proxy,
        fixture.transactions[2],
        9,
        1,
        r"^testForkBalRepeated\(\)$",
    );
    assert_test(&mut cmd, "testForkBalRepeated");
    proxy.assert_parent_bal(&fixture);
    assert_eq!(proxy.count(BAL_METHOD), 2, "unavailable BAL must retry, then reuse its success");
});

forgetest_async!(fork_bal_reuses_parent_seed_only_for_the_same_source, |prj, cmd| {
    let fixture = Fixture::new().await;
    let first = Proxy::new(&fixture, Response::Native).await;
    let second = Proxy::new(&fixture, Response::Native).await;
    prj.add_test("ForkBal.t.sol", TEST);
    command(
        &mut cmd,
        &fixture,
        &first,
        fixture.transactions[2],
        9,
        1,
        r"^testForkBalSeparateSources\(\)$",
    )
    .env("BAL_OTHER_RPC_URL", &second.endpoint);
    assert_test(&mut cmd, "testForkBalSeparateSources");
    for proxy in [&first, &second] {
        proxy.assert_parent_bal(&fixture);
        assert_eq!(proxy.count(BAL_METHOD), 1, "each source must prepare its own parent seed");
        assert_eq!(proxy.slot_reads(U256::ZERO), 0);
    }
});

forgetest_async!(fork_bal_inline_config_controls_runtime_requests, |prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = Proxy::new(&fixture, Response::Native).await;
    for (config_disabled, contract_override, function_override) in [
        (false, None, Some(true)),
        (false, Some(true), None),
        (true, None, Some(false)),
        (true, Some(false), None),
        (false, Some(true), Some(false)),
        (true, Some(false), Some(true)),
    ] {
        prj.update_config(|config| config.no_fork_bal = config_disabled);
        let mut source = TEST.to_owned();
        if let Some(disabled) = contract_override {
            source = source.replace(
                "contract ForkBalTest {",
                &format!(
                    "/// forge-config: default.no_fork_bal = {disabled}\ncontract ForkBalTest {{"
                ),
            );
        }
        if let Some(disabled) = function_override {
            source = source.replace(
                "    function testForkBal() public {",
                &format!(
                    "    /// forge-config: default.no_fork_bal = {disabled}\n    function testForkBal() public {{"
                ),
            );
        }
        prj.add_test("ForkBal.t.sol", &source);
        let disabled = function_override.or(contract_override).unwrap_or(config_disabled);
        for mode in 0..=3 {
            proxy.clear();
            command(
                &mut cmd,
                &fixture,
                &proxy,
                fixture.transactions[2],
                9,
                mode,
                r"^testForkBal\(\)$",
            );
            assert_test(&mut cmd, "testForkBal");
            assert_eq!(
                proxy.count(BAL_METHOD),
                usize::from(!disabled),
                "config={config_disabled}, contract={contract_override:?}, function={function_override:?}, mode={mode}"
            );
            if disabled {
                assert!(proxy.slot_reads(U256::ZERO) > 0);
            } else {
                proxy.assert_parent_bal(&fixture);
                assert_eq!(proxy.slot_reads(U256::ZERO), 0);
            }
        }
    }
});

forgetest_async!(fork_bal_setup_forks_keep_creation_policy_on_roll, |prj, cmd| {
    let fixture = Fixture::new().await;
    let proxy = Proxy::new(&fixture, Response::Native).await;
    for disabled in [false, true] {
        prj.update_config(|config| config.no_fork_bal = disabled);
        let source = TEST.replace(
            "    function testForkBal() public {",
            &format!(
                r#"
    uint256 setupActive;
    uint256 setupInactive;

    function setUp() public {{
        string memory url = vm.envString("BAL_RPC_URL");
        uint256 parent = vm.envUint("BAL_PARENT");
        setupActive = vm.createSelectFork(url, parent);
        setupInactive = vm.createFork(url, parent);
    }}

    /// forge-config: default.no_fork_bal = {}
    function testForkBalSetupRoll() public {{
        bytes32 target = vm.envBytes32("BAL_TARGET");
        if (vm.envUint("BAL_MODE") == 2) {{
            vm.rollFork(target);
        }} else {{
            vm.rollFork(setupInactive, target);
            require(vm.activeFork() == setupActive, "inactive roll selected the fork");
            require(value() == 7, "inactive roll changed active state");
            vm.selectFork(setupInactive);
        }}
        require(value() == 9, "wrong prefix state");
        require(uint256(vm.load(counter, bytes32(uint256(1)))) == 19, "lost read-only slot");
    }}

    function testForkBal() public {{"#,
                !disabled
            ),
        );
        prj.add_test("ForkBal.t.sol", &source);
        for mode in [2, 3] {
            proxy.clear();
            command(
                &mut cmd,
                &fixture,
                &proxy,
                fixture.transactions[2],
                9,
                mode,
                r"^testForkBalSetupRoll\(\)$",
            );
            assert_test(&mut cmd, "testForkBalSetupRoll");
            assert_eq!(
                proxy.count(BAL_METHOD),
                usize::from(!disabled),
                "disabled={disabled}, mode={mode}"
            );
            if disabled {
                assert!(proxy.slot_reads(U256::ZERO) > 0);
            } else {
                proxy.assert_parent_bal(&fixture);
                assert_eq!(proxy.slot_reads(U256::ZERO), 0);
            }
        }
    }
});
