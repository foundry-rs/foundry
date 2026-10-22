// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// forge-config: default.isolate = true
contract ForkSwitchIsolatedWarmthTest is Test {
    uint256 constant COLD_ACCOUNT_ACCESS = 2600;
    uint256 constant COLD_SLOAD = 2100;

    uint256 mainnetFork;
    uint256 optimismFork;
    uint256 written;

    function setUp() public {
        mainnetFork = vm.createFork("mainnet");
        optimismFork = vm.createFork("optimism");
    }

    function writeAndSwitch(uint256 forkId, bool shouldRevert) external {
        written = 1;
        vm.selectFork(forkId);
        if (shouldRevert) revert();
    }

    // An isolated call that switches forks replaces the running transaction's state with its own,
    // so warmth of the running transaction does not survive it, and the call's survives only if it
    // succeeds.
    function testForkRevertedSwitchDropsWarmth() public {
        vm.selectFork(mainnetFork);
        warmUp();
        try this.writeAndSwitch(optimismFork, true) {} catch {}

        assertTrue(isCold(address(0xBEEF)), "running transaction warmth survived");
        assertTrue(isCold(address(vm)), "reverted call warmth survived");
        assertTrue(isCold(address(this)), "reverted call warmth survived");
        assertTrue(isColdSlot(), "running transaction slot warmth survived");
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
