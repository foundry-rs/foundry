// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3190
contract Issue3190Test is Test {
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
