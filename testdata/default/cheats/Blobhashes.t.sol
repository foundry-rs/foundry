// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.25;

import "utils/Test.sol";

contract BlobhashesTest is Test {
    function testSetAndGetBlobhashes() public {
        bytes32[] memory blobhashes = new bytes32[](2);
        blobhashes[0] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000001);
        blobhashes[1] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000002);
        vm.blobhashes(blobhashes);

        bytes32[] memory gotBlobhashes = vm.getBlobhashes();
        assertEq(gotBlobhashes[0], blobhashes[0]);
        assertEq(gotBlobhashes[1], blobhashes[1]);
    }
}
