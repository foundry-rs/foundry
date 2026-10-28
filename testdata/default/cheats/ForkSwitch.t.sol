// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Slot {
    uint256 value;

    function set(uint256 newValue) external {
        value = newValue;
    }
}

/// forge-config: default.isolate = false
contract ForkSwitchStorageOriginalsTest is Test {
    // EIP-3529 refund for restoring a slot that was zero at transaction start.
    uint256 constant RESTORE_ZERO_REFUND = 19_900;
    address constant WETH = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;

    uint256 forkA;
    uint256 forkB;
    mapping(bool => uint256) values;
    // Contracts that exist only on fork A and are not persistent.
    mapping(bool => Slot) local;

    function setUp() public {
        forkA = vm.createSelectFork("mainnet");
        forkB = vm.createFork("mainnet");
        local[false] = new Slot();
        local[true] = new Slot();
    }

    function store(bool switching, uint256 value) internal returns (uint256 cost) {
        uint256 start = gasleft();
        values[switching] = value;
        cost = start - gasleft();
    }

    function roundTrip(bool switching) internal {
        if (switching) {
            vm.selectFork(forkB);
            vm.selectFork(forkA);
        }
    }

    // Writes a slot that is zero at transaction start, optionally switching forks between writes,
    // and returns the cost of each write after the first.
    function write(bool switching) external returns (uint256 second, uint256 third, uint256 restore) {
        // An isolated call caps its refund at a fifth of its gas, which must stay above the refund.
        uint256 start = gasleft();
        while (start - gasleft() < 5 * RESTORE_ZERO_REFUND) {}
        store(switching, 1);
        roundTrip(switching);
        second = store(switching, 2);
        roundTrip(switching);
        third = store(switching, 3);
        restore = store(switching, 0);
        roundTrip(switching);
    }

    // Fork switches keep the transaction-start originals of storage slots, so writes after a
    // switch are charged and refunded as without one.
    function testForkSwitchKeepsStorageOriginals() public {
        (uint256 second, uint256 third, uint256 restore) = this.write(false);
        assertEq(vm.lastCallGas().gasRefunded, int256(RESTORE_ZERO_REFUND), "control refund");

        (uint256 secondSwitched, uint256 thirdSwitched, uint256 restoreSwitched) = this.write(true);
        assertEq(vm.lastCallGas().gasRefunded, int256(RESTORE_ZERO_REFUND), "restore refund");
        assertEq(secondSwitched, second, "write after one round trip");
        assertEq(thirdSwitched, third, "write after two round trips");
        assertEq(restoreSwitched, restore, "restoring write");
        assertEq(valueCost(true), valueCost(false), "warmth after a round trip");
        assertEq(values[true], 0, "restoring write after a round trip");
    }

    function valueCost(bool switching) internal view returns (uint256) {
        uint256 slot;
        assembly {
            slot := values.slot
        }
        return sloadCost(uint256(keccak256(abi.encode(switching, slot))));
    }

    function dirty(bool switching) external {
        store(switching, 1);
        roundTrip(switching);
    }

    function rewrite(bool switching) external returns (uint256 cost) {
        // Warm the slot in both cases, which carries its warmth across the switch.
        require(values[switching] == 1);
        if (switching) vm.selectFork(forkB);
        cost = store(switching, 2);
        if (switching) vm.selectFork(forkA);
    }

    // A fork switch in an earlier call does not leave its originals behind for a later call that
    // switches to the same fork.
    function testForkSwitchKeepsLaterCallStorageOriginals() public {
        this.dirty(false);
        uint256 cost = this.rewrite(false);

        this.dirty(true);
        assertEq(this.rewrite(true), cost, "write after switching in a later call");
    }

    function writeLocal(bool switching) external returns (uint256 cost) {
        local[switching].set(1);
        roundTrip(switching);
        uint256 start = gasleft();
        local[switching].set(2);
        cost = start - gasleft();
    }

    // A fork keeps the originals of accounts that do not leave it.
    function testForkSwitchKeepsLocalStorageOriginals() public {
        assertEq(this.writeLocal(true), this.writeLocal(false), "write after a round trip");
    }

    function sloadCost(uint256 key) internal view returns (uint256 cost) {
        assembly {
            cost := gas()
            let value := sload(key)
            cost := sub(cost, gas())
            if eq(value, 0xdeadbeef) { revert(0, 0) }
        }
    }

    // Reads a slot of this persistent contract on fork A, then another one and a slot of WETH,
    // which is local to fork B, on fork B.
    function readAcrossForks() external returns (uint256 onA, uint256 persistentOnB, uint256 localOnB) {
        onA = sloadCost(0x1234);
        vm.selectFork(forkB);
        persistentOnB = sloadCost(0x1235);
        uint256 start = gasleft();
        (bool success,) = WETH.staticcall(abi.encodeWithSignature("balanceOf(address)", address(this)));
        require(success);
        localOnB = start - gasleft();
        vm.selectFork(forkA);
    }

    // A persistent slot an earlier call read on a fork is as warm in a later call as one it read
    // before switching, while a fork-local account stays warm on its fork.
    function testForkSwitchKeepsLaterCallPersistentWarmth() public {
        (,, uint256 localCost) = this.readAcrossForks();
        (uint256 onA, uint256 persistentOnB, uint256 localOnB) = this.readAcrossForks();
        assertEq(persistentOnB, onA, "persistent slot warmth");
        assertEq(localCost - localOnB, 4500, "fork-local account and slot warmth");
    }

    function readAfterRevertAndPersist() external returns (uint256 persistentOnB, uint256 localSaved) {
        uint256 snapshot = vm.snapshotState();
        (,, uint256 localCost) = this.readAcrossForks();
        vm.revertToState(snapshot);
        vm.makePersistent(WETH);
        uint256 localOnB;
        (, persistentOnB, localOnB) = this.readAcrossForks();
        localSaved = localCost - localOnB;
    }

    // Within a call, a fork keeps the warmth of persistent slots it read across a snapshot revert
    // and of accounts made persistent since.
    function testForkSwitchKeepsPersistentWarmthWithinCall() public {
        (uint256 persistentOnB, uint256 localSaved) = this.readAfterRevertAndPersist();
        assertEq(persistentOnB, 110, "persistent slot warmth after a snapshot revert");
        assertEq(localSaved, 4500, "warmth of an account made persistent");
    }
}

/// forge-config: default.isolate = true
contract ForkSwitchStorageOriginalsIsolatedTest is ForkSwitchStorageOriginalsTest {}

/// Forks the switch warmth tests move between, and their account warmth probe.
abstract contract ForkSwitchWarmthBase is Test {
    uint256 mainnetFork;
    uint256 optimismFork;

    function setUp() public {
        mainnetFork = vm.createFork("mainnet");
        optimismFork = vm.createFork("optimism");
    }

    function isCold(address account) internal view returns (bool) {
        uint256 cost;
        assembly {
            cost := gas()
            let accountBalance := balance(account)
            cost := sub(cost, gas())
            if eq(accountBalance, 0xdeadbeef) { revert(0, 0) }
        }
        return cost > 2000;
    }
}

/// forge-config: default.isolate = true
contract ForkSwitchIsolatedWarmthTest is ForkSwitchWarmthBase {
    uint256 constant COLD_ACCOUNT_ACCESS = 2600;
    uint256 constant COLD_SLOAD = 2100;

    uint256 written;

    function writeAndSwitch(uint256 forkId, bool shouldRevert) external {
        written = 1;
        vm.selectFork(forkId);
        if (shouldRevert) revert();
    }

    // An isolated call that switches forks replaces the running transaction's state with its own,
    // so warmth of the running transaction does not survive it, and the call's accounts stay warm
    // only if it succeeds. Slots the call accessed before switching stay warm even if it reverts.
    function testForkRevertedSwitchDropsWarmth() public {
        vm.selectFork(mainnetFork);
        warmUp();
        try this.writeAndSwitch(optimismFork, true) {} catch {}

        assertTrue(isCold(address(0xBEEF)), "running transaction warmth survived");
        assertTrue(isCold(address(vm)), "reverted call warmth survived");
        assertTrue(isCold(address(this)), "reverted call warmth survived");
        assertTrue(isColdSlot(), "running transaction slot warmth survived");
        assertTrue(!isWrittenCold(), "reverted call slot warmth was dropped");
    }

    function testForkSucceededSwitchKeepsCallWarmth() public {
        vm.selectFork(mainnetFork);
        warmUp();
        this.writeAndSwitch(optimismFork, false);

        assertTrue(isCold(address(0xBEEF)), "running transaction warmth survived");
        assertTrue(!isCold(address(vm)), "call warmth was dropped");
        assertTrue(!isCold(address(this)), "call warmth was dropped");
        assertTrue(!isWrittenCold(), "call slot warmth was dropped");
    }

    // Selecting the original fork again after a reverted switch accesses the cheatcode address and
    // the fork ID slot cold.
    function testForkRevertedSwitchSelectBackPaysColdAccess() public {
        vm.selectFork(mainnetFork);
        try this.writeAndSwitch(optimismFork, true) {} catch {}

        uint256 gasBefore = gasleft();
        vm.selectFork(mainnetFork);
        assertGt(gasBefore - gasleft(), COLD_ACCOUNT_ACCESS + COLD_SLOAD, "select back was warm");
    }

    function warmUp() internal view {
        isCold(address(0xBEEF));
        isCold(address(vm));
        isColdSlot();
    }

    function isColdSlot() internal view returns (bool) {
        uint256 cost;
        assembly {
            cost := gas()
            let value := sload(mainnetFork.slot)
            cost := sub(cost, gas())
            if eq(value, 0xdeadbeef) { revert(0, 0) }
        }
        return cost > 2000;
    }

    function isWrittenCold() internal view returns (bool) {
        uint256 cost;
        assembly {
            cost := gas()
            let value := sload(written.slot)
            cost := sub(cost, gas())
            if eq(value, 0xdeadbeef) { revert(0, 0) }
        }
        return cost > 2000;
    }
}

/// forge-config: default.isolate = false
contract ForkSwitchWarmthRoundTripTest is ForkSwitchWarmthBase {
    // DAI, whose `totalSupply()` reads a storage slot.
    address constant DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;

    // A fork keeps the warmth of the running transaction across later switches, including visits
    // that do not touch the warm account or slot again.
    function testForkWarmthSurvivesUntouchedVisit() public {
        vm.selectFork(mainnetFork);
        address account = address(0xBEEF);
        isCold(account);
        isColdSlot();

        vm.selectFork(optimismFork);
        vm.selectFork(mainnetFork);
        vm.selectFork(optimismFork);
        vm.selectFork(mainnetFork);

        assertTrue(!isCold(account), "account warmth was dropped");
        assertTrue(!isColdSlot(), "slot warmth was dropped");
    }

    function isColdSlot() internal view returns (bool) {
        uint256 cost;
        address dai = DAI;
        assembly {
            mstore(0, 0x18160ddd) // totalSupply()
            cost := gas()
            let success := staticcall(gas(), dai, 28, 4, 0, 32)
            cost := sub(cost, gas())
            if iszero(success) { revert(0, 0) }
        }
        // Reading the slot of a warm account through a warm call costs well under a cold access.
        return cost > 2000;
    }
}

// A frame that fails after a callee restored a snapshot and selected another fork reinstates the
// source fork's state the restoration replaced, while the other fork stays selected.
/// forge-config: default.isolate = false
contract ForkSwitchSnapshotRestoreTest is Test {
    address payable constant TARGET = payable(address(0x777));

    function testForkRestoreThenSwitchInRevertedCall() public {
        uint256 source = vm.createFork("mainnet");
        uint256 other = vm.createFork("mainnet");
        vm.selectFork(other);
        vm.deal(TARGET, 200);
        vm.selectFork(source);
        vm.deal(TARGET, 50);
        vm.deal(address(this), 1000);
        uint256 snapshotId = vm.snapshotState();

        (bool ok,) = address(this).call(abi.encodeCall(this.runRestore, (snapshotId, other)));
        require(!ok, "runner succeeded");
        assertEq(vm.activeFork(), other, "selected fork changed");
        assertEq(TARGET.balance, 50, "restoration was not undone");
    }

    function runRestore(uint256 snapshotId, uint256 fork) external {
        this.restoreAndSwitch(snapshotId, fork);
        revert("runner reverted");
    }

    function restoreAndSwitch(uint256 snapshotId, uint256 fork) external {
        TARGET.transfer(10);
        require(vm.revertToState(snapshotId), "restore failed");
        vm.selectFork(fork);
        TARGET.transfer(17);
    }
}

/// forge-config: default.isolate = true
contract ForkSwitchSnapshotRestoreIsolatedTest is ForkSwitchSnapshotRestoreTest {}

contract ForkSwitchBox {
    uint256 public value = 7;
    uint256 public other = 3;

    function set(uint256 next) external returns (uint256 previous) {
        previous = value;
        value = next;
    }

    function kill() external {
        selfdestruct(payable(address(0)));
    }
}

// A contract created in the running transaction keeps its storage across fork round trips and
// isolated calls, and stays created there, so EIP-6780 `SELFDESTRUCT` still deletes it.
/// forge-config: default.isolate = false
contract ForkSwitchCreatedAccountTest is Test {
    uint256 a;
    uint256 b;

    function setUp() public {
        a = vm.createFork("mainnet");
        b = vm.createFork("mainnet");
        vm.selectFork(a);
    }

    function roundTrip() internal {
        vm.selectFork(b);
        vm.selectFork(a);
    }

    function testForkCallAfterSwitch() public {
        ForkSwitchBox box = new ForkSwitchBox();
        assertEq(box.set(8), 7);
        roundTrip();
        assertEq(box.set(9), 8);
        assertEq(box.other(), 3);
    }

    function testForkTwoSwitches() public {
        ForkSwitchBox box = new ForkSwitchBox();
        assertEq(box.set(8), 7);
        roundTrip();
        assertEq(box.value(), 8);
        roundTrip();
        assertEq(box.value(), 8);
        assertEq(box.other(), 3);
    }
}

/// forge-config: default.isolate = true
contract ForkSwitchCreatedAccountIsolatedTest is ForkSwitchCreatedAccountTest {
    // Only an isolated call ends its transaction, which deletes the contract.
    function testForkKillAfterSwitch() public {
        ForkSwitchBox box = new ForkSwitchBox();
        roundTrip();
        box.kill();
        assertEq(address(box).code.length, 0);
    }
}
