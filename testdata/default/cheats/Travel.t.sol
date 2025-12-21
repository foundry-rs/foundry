// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract ChainIdTest is Test {
    function testChainId() public {
        vm.chainId(10);
        assertEq(block.chainid, 10, "chainId switch failed");
    }

    function testChainIdFuzzed(uint64 chainId) public {
        vm.chainId(chainId);
        assertEq(block.chainid, chainId, "chainId switch failed");
    }
}
