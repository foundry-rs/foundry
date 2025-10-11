// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SetBlockhash is Test {
    function testSetBlockhash() public {
        bytes32 blockHash = 0x1234567890123456789012345678901234567890123456789012345678901234;
        vm.setBlockhash(block.number - 1, blockHash);
        bytes32 expected = blockhash(block.number - 1);
        assertEq(blockHash, expected);
    }
}
