//! OSS-832 fork rollback acceptance cases, including the selected corrections to REVM master.

use anvil::{NodeConfig, NodeHandle, spawn};
use foundry_config::{RpcEndpointUrl, RpcEndpoints};
use foundry_test_utils::{TestCommand, TestProject};

const SOURCE: &str = r#"
interface Vm {
    function createFork(string calldata, uint256) external returns (uint256);
    function createSelectFork(string calldata, uint256) external returns (uint256);
    function rollFork(uint256) external;
    function selectFork(uint256) external;
    function activeFork() external view returns (uint256);
    function makePersistent(address) external;
    function rpcUrl(string calldata) external view returns (string memory);
    function snapshotState() external returns (uint256);
    function revertToState(uint256) external returns (bool);
    function registerSstoreHook(address, bytes4) external;
}

Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

contract Store {
    uint256 public v;
    mapping(uint256 => uint256) public slots;

    function set(uint256 x) external { v = x; }

    function fill(uint256 n) external {
        for (uint256 i = 1; i <= n; i++) slots[i] = i;
    }
}

contract SwitchHelper {
    uint256 public v;

    function run(uint256 target, bool fail) external {
        v = 7;
        vm.selectFork(target);
        for (uint256 i = 1; i <= 10; i++) v = i;
        if (fail) revert("boom");
    }

    function rollAndRevert(uint256 blockNumber) external {
        v = 7;
        vm.rollFork(blockNumber);
        v = 8;
        revert("boom");
    }

    function createSelectAndRevert() external {
        v = 7;
        vm.createSelectFork(vm.rpcUrl("local"), 2);
        v = 8;
        revert("boom");
    }

    function roundTrip(uint256 other, uint256 home, Store s) external {
        s.set(3);
        vm.selectFork(other);
        v = 9;
        vm.selectFork(home);
        s.set(4);
        revert("boom");
    }

    function outer(uint256 first, uint256 second) external {
        vm.selectFork(first);
        v = 5;
        try this.inner(second) {} catch {}
        v += 1;
    }

    function inner(uint256 target) external {
        vm.selectFork(target);
        v = 99;
        revert("inner");
    }

    function snapshotRoundTrip(uint256 home, uint256 other, Store s) external {
        s.set(1);
        vm.selectFork(other);
        uint256 id = vm.snapshotState();
        vm.selectFork(home);
        s.set(2);
        vm.selectFork(other);
        vm.revertToState(id);
        vm.selectFork(home);
        revert("boom");
    }

    uint256 hookHome;
    uint256 hookOther;
    Store hookStore;

    function hookRoundTrip(uint256 home, uint256 other, Store hooked, Store s) external {
        (hookHome, hookOther, hookStore) = (home, other, s);
        vm.registerSstoreHook(address(hooked), this.onStore.selector);
        hooked.set(1);
        vm.selectFork(other);
        revert("boom");
    }

    function onStore(address, bytes32, bytes32, bytes32) external {
        require(msg.sender == address(vm), "only vm");
        // Warm accounts that the hook cleanup removes from the journal afterwards.
        require(address(0xdead01).balance + address(0xdead02).balance == 0);
        vm.selectFork(hookOther);
        hookStore.set(5);
        vm.selectFork(hookHome);
    }

    uint256 public x;

    function firstSelectionRevert(uint256 first, uint256 second, bool innerRevert) external {
        x = 1;
        if (innerRevert) {
            try this.selectAndRevert(first) {} catch {}
        } else {
            vm.selectFork(first);
        }
        vm.selectFork(second);
        revert("boom");
    }

    function selectAndRevert(uint256 target) external {
        vm.selectFork(target);
        revert("inner");
    }
}

contract SelectForkRevertForkTest {
    uint256 a;
    uint256 b;
    uint256 marker;
    SwitchHelper h;
    Store filler;
    Store home;
    Store hooked;
    Store onB;

    function setUp() public {
        a = vm.createFork(vm.rpcUrl("local"), 1);
        b = vm.createFork(vm.rpcUrl("local"), 2);
        vm.selectFork(b);
        onB = new Store();
        vm.selectFork(a);
        h = new SwitchHelper();
        vm.makePersistent(address(h));
        filler = new Store();
        vm.makePersistent(address(filler));
        home = new Store();
        hooked = new Store();
        marker = 42;
    }

    function checkOnB() internal view {
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
        require(vm.activeFork() == b, "active fork");
        require(h.v() == 7, "persistent helper");
    }

    function testForkSwitchKeptOnSuccess() public {
        h.run(b, false);
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
        require(vm.activeFork() == b, "active fork");
        require(h.v() == 10, "persistent helper");
    }

    function testForkRevertUndoesOnlyPostSwitchWrites() public {
        try h.run(b, true) {} catch {}
        checkOnB();
    }

    function testForkRevertAfterLongActiveJournal() public {
        filler.fill(60);
        try h.run(b, true) {} catch {}
        checkOnB();
        require(filler.slots(1) == 1 && filler.slots(60) == 60, "filler");
    }

    function testForkRevertAfterLongTargetJournal() public {
        vm.selectFork(b);
        Store onB = new Store();
        onB.fill(60);
        vm.selectFork(a);
        try h.run(b, true) {} catch {}
        checkOnB();
        require(address(onB).code.length > 0, "contract deployed on B");
        require(onB.slots(1) == 1 && onB.slots(60) == 60, "storage written on B");
    }

    function testForkRevertAfterActiveRoll() public {
        try h.rollAndRevert(2) {} catch {}
        require(vm.activeFork() == a && block.number == 2, "rolled fork");
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
        require(h.v() == 7, "persistent helper");
    }

    function testForkRevertAfterCreateSelectFork() public {
        try h.createSelectAndRevert() {} catch {}
        require(vm.activeFork() == 2, "active fork");
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
        require(h.v() == 7, "persistent helper");
    }

    function testForkRevertAfterRoundTrip() public {
        home.set(1);
        try h.roundTrip(b, a, home) {} catch {}
        require(vm.activeFork() == a, "active fork");
        require(home.v() == 1, "writes on A");
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
    }

    function testForkNestedSwitchInnerRevert() public {
        h.outer(b, a);
        require(vm.activeFork() == a, "active fork");
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
        require(h.v() == 6, "persistent helper on A");
        vm.selectFork(b);
        require(h.v() == 6, "persistent helper on B");
    }

    function testForkRevertAfterSnapshotRestore() public {
        try h.snapshotRoundTrip(a, b, home) {} catch {}
        require(vm.activeFork() == a, "active fork");
        require(home.v() == 0, "writes on A");
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
    }

    function testForkRevertAfterStorageHookCleanup() public {
        try h.hookRoundTrip(a, b, hooked, onB) {} catch {}
        require(vm.activeFork() == b, "active fork");
        require(onB.v() == 0, "writes on B");
        require(a == 0 && b == 1 && marker == 42, "test contract storage");
    }
}

contract SelectForkFirstSelectionForkTest {
    uint256 a;
    uint256 b;
    SwitchHelper h;

    function setUp() public {
        h = new SwitchHelper();
        a = vm.createFork(vm.rpcUrl("local"), 1);
        b = vm.createFork(vm.rpcUrl("local"), 2);
    }

    function check() internal {
        require(vm.activeFork() == b, "active fork");
        require(h.x() == 0, "write before the first selection on B");
        vm.selectFork(a);
        require(h.x() == 0, "write before the first selection on A");
    }

    function testForkRevertUndoesWritesBeforeFirstSelection() public {
        try h.firstSelectionRevert(a, b, false) {} catch {}
        check();
    }

    function testForkRevertAfterRevertedFirstSelection() public {
        try h.firstSelectionRevert(a, b, true) {} catch {}
        check();
    }
}
contract SelectForkRevertedCallerForkTest {
    uint256 a;
    uint256 b;
    SwitchHelper h;
    Store sA;

    function setUp() public {
        a = vm.createFork(vm.rpcUrl("local"), 1);
        b = vm.createFork(vm.rpcUrl("local"), 2);
        vm.selectFork(a);
        h = new SwitchHelper();
        vm.makePersistent(address(h));
        sA = new Store();
    }

    // The switch made by a successful call persists when its caller reverts, along with the writes
    // made on the source fork before the switch. Only the writes made on the new fork are undone.
    function testForkRevertAfterSwitchInSuccessfulCall() public {
        try this.switchAndRevert(b) {} catch {}
        require(vm.activeFork() == b, "active fork");
        require(h.v() == 7, "persistent helper on B");
        vm.selectFork(a);
        require(h.v() == 7, "persistent helper on A");
        require(sA.v() == 11, "writes on A");
    }

    function switchAndRevert(uint256 target) external {
        sA.set(11);
        h.run(target, false);
        revert("boom");
    }
}
"#;

/// Starts a node with two blocks to fork from and points the `local` endpoint at it.
async fn setup(prj: &TestProject, isolate: bool) -> NodeHandle {
    let (api, anvil) = spawn(NodeConfig::test()).await;
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();
    let endpoint = anvil.http_endpoint();
    prj.add_source(
        "SelectForkRevert.t.sol",
        &SOURCE.replace("vm.rpcUrl(\"local\")", &format!("{endpoint:?}")),
    );
    prj.update_config(|config| {
        config.isolate = isolate;
        config.rpc_endpoints = RpcEndpoints::new([("local", RpcEndpointUrl::Url(endpoint))]);
    });
    anvil
}

fn assert_passes(cmd: &mut TestCommand) {
    // Run all thirteen scenarios together so a failure cannot hide later completion gates.
    cmd.args(["test", "--match-contract", "SelectFork.*ForkTest"]).assert_success();
}

// A frame that switched forks and reverted only undoes the writes made on the new fork after the
// switch, not entries the new fork recorded in earlier transactions.
#[forgetest]
async fn ethereum_native_fork_revert_matrix_isolated(prj: _, cmd: _) {
    let _node = setup(&prj, true).await;
    assert_passes(&mut cmd);
}

#[forgetest]
async fn ethereum_native_fork_revert_matrix_not_isolated(prj: _, cmd: _) {
    let _node = setup(&prj, false).await;
    assert_passes(&mut cmd);
}
