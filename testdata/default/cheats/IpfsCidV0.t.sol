// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract IpfsCidV0Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testIpfsCidV0() public {
        string memory filePath = "testdata/fixtures/File/test.txt";

        bytes32 cid = vm.ipfsCidV0(filePath);

        bytes32 expectedCid = 0x94da694df5cf2e139206cddcdd6f855baa45e519c5fdbc2e6aa1cf803cfd65d5;

        assertEq(cid, expectedCid);
    }
}
