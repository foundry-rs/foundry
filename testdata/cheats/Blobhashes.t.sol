// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract AddrTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSetBlobhashes() public {
        vm.blobhashes([bytes32(1), bytes32(2)]);
        assertEq(blobhash(0), bytes32(1));
        assertEq(blobhash(0), bytes32(2));
    }
}
