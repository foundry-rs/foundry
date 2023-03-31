// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3055
contract Issue3055Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function test_snapshot() external {
        uint256 snapId = vm.snapshot();
        assertEq(uint256(0), uint256(1));
        vm.revertTo(snapId);
    }

    function test_snapshot2() public {
        uint256 snapshot = vm.snapshot();
        assertTrue(false);
        vm.revertTo(snapshot);
        assertTrue(true);
    }
}
