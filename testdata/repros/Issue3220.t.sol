// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3220
contract Issue3220Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    uint256 fork1;
    uint256 fork2;
    uint256 counter;

    function setUp() public {
        fork1 = vm.createFork("rpcAlias", 7475589);
        vm.selectFork(fork1);
        fork2 = vm.createFork("rpcAlias", 12880747);
    }

    function testForkRevert() public {
        vm.selectFork(fork2);
        vm.selectFork(fork1);

        // do a bunch of work to increase the revm checkpoint counter
        for (uint256 i = 0; i < 10; i++) {
            mockCount();
        }

        vm.selectFork(fork2);

        vm.expectRevert("This fails");
        doRevert();
    }

    function doRevert() public {
        revert("This fails");
    }

    function mockCount() public {
        counter += 1;
    }
}
