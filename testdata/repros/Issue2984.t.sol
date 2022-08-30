// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/2984
contract Issue2984Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);
    uint256 fork;
    uint256 snapshot;

    function setUp() public {
        fork = vm.createSelectFork("https://api.avax-test.network/ext/bc/C/rpc", 12880747);
        snapshot = vm.snapshot();
    }

    function testForkRevertSnapshot() public {
        vm.revertTo(snapshot);
    }

    function testForkSelectSnapshot() public {
        uint256 fork2 = vm.createSelectFork("https://api.avax-test.network/ext/bc/C/rpc", 12880749);
    }
}
