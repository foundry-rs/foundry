// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6355
contract Issue6355Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 snapshot;
    Target targ;

    function setUp() public {
        snapshot = vm.snapshot();
        targ = new Target();
    }

    // this non-deterministically fails sometimes and passes sometimes
    function test_shouldPass() public {
        assertEq(2, targ.num());
    }

    // always fails
    function test_shouldFailWithRevertTo() public {
        assertEq(3, targ.num());
        vm.revertTo(snapshot);
    }

    // always fails
    function test_shouldFail() public {
        assertEq(3, targ.num());
    }
}

contract Target {
    function num() public pure returns (uint256) {
        return 2;
    }
}
