//! Access list prefetching of fork state under `fork_prefetch`.

use alloy_primitives::{Address, B256, U256, address};
use anvil::{NodeConfig, spawn};
use foundry_test_utils::{rpc::spawn_rpc_proxy_recording_method, str};
use serde_json::Value;

/// A contract on the fork whose code sums its first [`SLOT_COUNT`] storage slots.
const SLOTS: Address = address!("0x000000000000000000000000000000000000510c");
const SLOT_COUNT: u8 = 40;
/// A contract on the fork whose code returns its slot 0.
const SLOT_ZERO: Address = address!("0x000000000000000000000000000000000000510d");
/// Where forge deploys the test contract.
const TEST_CONTRACT: Address = address!("0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496");

/// Runtime code that returns the sum of the first [`SLOT_COUNT`] storage slots.
fn sum_slots_code() -> Vec<u8> {
    let mut code = vec![0x60, 0x00];
    for slot in 0..SLOT_COUNT {
        // PUSH1 slot, SLOAD, ADD.
        code.extend([0x60, slot, 0x54, 0x01]);
    }
    // PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN.
    code.extend([0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);
    code
}

/// Runtime code that returns storage slot 0.
const SLOT_ZERO_CODE: [u8; 11] = [0x60, 0x00, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3];

const TEST: &str = r#"
interface Vm {
    function envString(string calldata) external returns (string memory);
    function envUint(string calldata) external returns (uint256);
    function createSelectFork(string calldata, uint256) external returns (uint256);
    function rpc(string calldata, string calldata) external returns (bytes memory);
}

contract ForkPrefetchTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    address constant SLOTS = 0x000000000000000000000000000000000000510c;

    function setUp() public {
        vm.createSelectFork(vm.envString("PREFETCH_RPC_URL"), vm.envUint("PREFETCH_BLOCK"));
    }

    function test_sum() public {
        (bool ok, bytes memory data) = SLOTS.call("");
        require(ok, "call");
        require(abi.decode(data, (uint256)) == 820, "sum");
    }

    function test_refresh() public {
        test_sum();
        // The simulation of this test prefetched slot 0 of SLOT_ZERO with the old value.
        vm.rpc(
            "anvil_setStorageAt",
            '["0x000000000000000000000000000000000000510d","0x0","0x0000000000000000000000000000000000000000000000000000000000000002"]'
        );
        (bool ok, bytes memory data) = address(0x510d).call("");
        require(ok, "call");
        require(abi.decode(data, (uint256)) == 2, "refreshed");
    }
}
"#;

fn is_address(value: &Value, address: Address) -> bool {
    value.as_str().is_some_and(|value| value.eq_ignore_ascii_case(&address.to_string()))
}

#[forgetest]
async fn fork_prefetch_simulates_the_test_and_its_calls(prj: _, cmd: _) {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_code(SLOTS, sum_slots_code().into()).await.unwrap();
    for slot in 0..SLOT_COUNT {
        api.anvil_set_storage_at(SLOTS, U256::from(slot), B256::with_last_byte(slot + 1))
            .await
            .unwrap();
    }
    api.anvil_set_code(SLOT_ZERO, SLOT_ZERO_CODE.into()).await.unwrap();
    api.anvil_set_storage_at(SLOT_ZERO, U256::ZERO, B256::with_last_byte(1)).await.unwrap();
    api.mine_one().await.unwrap();
    let block = api.block_number().unwrap();
    let (rpc, simulations) =
        spawn_rpc_proxy_recording_method(handle.http_endpoint(), "eth_createAccessList").await;

    prj.add_test("ForkPrefetch.t.sol", TEST);
    cmd.env("PREFETCH_RPC_URL", &rpc);
    cmd.env("PREFETCH_BLOCK", block.to_string());
    cmd.args(["test", "--match-test", "test_sum", "--fork-prefetch", "--no-storage-caching"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ForkPrefetch.t.sol:ForkPrefetchTest
[PASS] test_sum() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // A slot that the endpoint changes after a prefetch read it is read again.
    cmd.forge_fuse();
    cmd.env("PREFETCH_RPC_URL", &rpc);
    cmd.env("PREFETCH_BLOCK", block.to_string());
    cmd.args(["test", "--match-test", "test_refresh", "--fork-prefetch", "--no-storage-caching"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ForkPrefetch.t.sol:ForkPrefetchTest
[PASS] test_refresh() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // The test function is simulated with the test contract sent as a state override, since it
    // exists only locally, and the call into the slots contract is simulated as is.
    let simulations = simulations.lock().unwrap();
    let test_call = simulations
        .iter()
        .find(|params| is_address(&params[0]["to"], TEST_CONTRACT))
        .unwrap_or_else(|| panic!("no simulation of the test function in {simulations:?}"));
    let overrides = test_call[2].as_object().expect("state overrides");
    let test_contract = overrides
        .iter()
        .find(|(address, _)| address.eq_ignore_ascii_case(&TEST_CONTRACT.to_string()))
        .map(|(_, account)| account)
        .expect("test contract override");
    assert!(test_contract["code"].is_string(), "{test_contract}");
    assert!(simulations.iter().any(|params| is_address(&params[0]["to"], SLOTS)));
}
