// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";
import "../logs/console.sol";

// https://github.com/foundry-rs/foundry/issues/3190
contract Issue3190Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function setUp() public {
        vm.chainId(99);
        assertEq(99, block.chainid);
    }

    function testChainId() public {
        assertEq(99, block.chainid);
        vm.chainId(100);
        assertEq(100, block.chainid);
    }
}
