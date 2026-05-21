// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract FeeTest is Test {
    function testFee() public {
        vm.fee(10);
        assertEq(block.basefee, 10, "fee failed");
    }

    function testFeeFuzzed(uint64 fee) public {
        vm.fee(fee);
        assertEq(block.basefee, fee, "fee failed");
    }
}

/// `vm.fee` must remain visible to a *called* contract under `--isolate`,
/// where Foundry zeroes `block.basefee` for the synthetic inner transaction
/// used for fee accounting. Regression test for #7277.
/// forge-config: default.isolate = true
contract IsolatedFeeTest is Test {
    BaseFeeRecorder internal recorder;

    function setUp() public {
        recorder = new BaseFeeRecorder();
    }

    function test_fee_visible_in_called_contract() public {
        vm.fee(456 gwei);
        recorder.record();
        assertEq(recorder.lastBaseFee(), 456 gwei);
    }
}

contract BaseFeeRecorder {
    uint256 public lastBaseFee;

    function record() external {
        lastBaseFee = block.basefee;
    }
}

/// `vm.snapshotState` / `vm.revertToState` must roll back the cheatcode-side
/// `EnvOverrides` for `vm.fee` in lockstep with the backend snapshot,
/// otherwise the BASEFEE opcode (rewritten in `step_end` from the override)
/// keeps returning the post-snapshot value even after `revertToState` rolls
/// back the underlying `EvmEnv`. Regression test for the review on PR
/// #14493.
contract FeeSnapshotRevertTest is Test {
    function test_fee_revert_to_state_clears_override() public {
        vm.fee(1000);
        uint256 id = vm.snapshotState();
        vm.fee(2000);
        assertEq(block.basefee, 2000, "pre-revert override not applied");
        assertTrue(vm.revertToState(id), "revertToState failed");
        assertEq(block.basefee, 1000, "override leaked past revertToState");
    }

    function test_fee_revert_to_state_restores_prior_override() public {
        uint256 id = vm.snapshotState();
        vm.fee(2000);
        assertEq(block.basefee, 2000, "override not applied before revert");
        assertTrue(vm.revertToState(id), "revertToState failed");
        // Before the snapshot no override was set, so BASEFEE should fall
        // back to the underlying env value (which `revert_state` restores).
        assertTrue(block.basefee != 2000, "override leaked past revertToState");
    }
}

/// `vm.fee` overrides must be scoped to the fork on which they were set and must
/// not bleed into other forks when `vm.selectFork` / `vm.createSelectFork`
/// switches the active fork.
contract MultiForkFeeIsolationTest is Test {
    // Use a sentinel basefee that will never occur on any real mainnet block.
    uint64 constant SENTINEL_FEE = 12_345 gwei;
    uint256 constant FORK_BLOCK = 14_608_400;

    function test_fee_override_does_not_bleed_across_forks() public {
        uint256 forkA = vm.createSelectFork("mainnet", FORK_BLOCK);
        vm.fee(SENTINEL_FEE);
        assertEq(block.basefee, SENTINEL_FEE, "override not active on forkA");

        // Switching to a second fork must not carry forkA's override along.
        vm.createSelectFork("mainnet2", FORK_BLOCK + 1);
        assertTrue(block.basefee != SENTINEL_FEE, "forkA fee override bled into forkB");

        // Switching back to forkA must restore its override.
        vm.selectFork(forkA);
        assertEq(block.basefee, SENTINEL_FEE, "forkA override lost after switching back");
    }
}

/// Same regression as `FeeSnapshotRevertTest`, but exercised under `--isolate`
/// where `vm.fee` only writes the override (the real `block.basefee` is left
/// untouched), so the snapshot/revert path is the only thing that can roll
/// the override back.
/// forge-config: default.isolate = true
contract IsolatedFeeSnapshotRevertTest is Test {
    BaseFeeRecorder internal recorder;

    function setUp() public {
        recorder = new BaseFeeRecorder();
    }

    function test_fee_revert_to_state_clears_override_in_isolation() public {
        vm.fee(1000);
        uint256 id = vm.snapshotState();
        vm.fee(2000);
        recorder.record();
        assertEq(recorder.lastBaseFee(), 2000, "pre-revert override not seen by call");
        assertTrue(vm.revertToState(id), "revertToState failed");
        recorder.record();
        assertEq(recorder.lastBaseFee(), 1000, "override leaked past revertToState");
    }
}
