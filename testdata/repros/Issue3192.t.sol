// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3192
contract Issue3192Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    uint256 fork1;
    uint256 fork2;

    function setUp() public {
        fork1 = vm.createFork("rpcAlias", 7475589);
        fork2 = vm.createFork("rpcAlias", 12880747);
        vm.selectFork(fork1);
    }

    function testForkSwapSelect() public {
        assertEq(fork1, vm.activeFork());
        vm.selectFork(fork2);
    }
}
