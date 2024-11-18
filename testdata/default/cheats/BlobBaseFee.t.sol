// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.25;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract BlobBaseFeeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_blob_base_fee() public {
        vm.blobBaseFee(6969);
        assertEq(vm.getBlobBaseFee(), 6969);
    }
}
