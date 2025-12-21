// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract ChainIdTest is Test {
    function testChainId() public {
        uint256 newChainId = 99;
        vm.chainId(newChainId);
        assertEq(newChainId, block.chainid);
    }
}
