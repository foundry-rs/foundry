// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.25;

import "utils/Test.sol";

contract BlobBaseFeeReader {
    function blobBaseFee() external view returns (uint256) {
        return block.blobbasefee;
    }
}

contract BlobBaseFeeTest is Test {
    BlobBaseFeeReader reader = new BlobBaseFeeReader();

    function test_blob_base_fee() public {
        vm.blobBaseFee(6969);
        assertEq(vm.getBlobBaseFee(), 6969);
        assertEq(reader.blobBaseFee(), 6969);
    }

    function test_blob_base_fee_gwei() public {
        vm.blobBaseFee(1 gwei);
        assertEq(vm.getBlobBaseFee(), 1 gwei);
        assertEq(reader.blobBaseFee(), 1 gwei);
    }

    function test_blob_base_fee_max() public {
        vm.blobBaseFee(type(uint128).max);
        assertEq(vm.getBlobBaseFee(), type(uint128).max);
        assertEq(reader.blobBaseFee(), type(uint128).max);
    }

    function test_blob_base_fee_too_large() public {
        vm._expectCheatcodeRevert("vm.blobBaseFee: blob base fee must be less than 2^128");
        vm.blobBaseFee(uint256(type(uint128).max) + 1);
    }
}
