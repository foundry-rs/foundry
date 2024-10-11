// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract EnsNamehashTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testEnsNamehash() public {
        assertEq(vm.ensNamehash(""), 0x0000000000000000000000000000000000000000000000000000000000000000);
        assertEq(vm.ensNamehash("eth"), 0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae);
        assertEq(vm.ensNamehash("foo.eth"), 0xde9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f);
    }
}
