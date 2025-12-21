// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.25;

import "utils/Test.sol";

contract BlobBaseFeeTest is Test {
    function test_blob_base_fee() public {
        vm.blobBaseFee(6969);
        assertEq(vm.getBlobBaseFee(), 6969);
    }
}
