// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.17;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import "../Counter.sol";

// Same as GetCode.t.sol but for 0.8.17 version
contract GetCodeTest17 is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetCodeMultiVersion() public {
        assertEq(vm.getCode("Counter.sol"), type(Counter).creationCode);
        require(
            keccak256(vm.getCode("Counter.sol")) != keccak256(vm.getCode("Counter.sol:Counter:0.8.18")),
            "Invalid artifact"
        );
        assertEq(vm.getCode("Counter.sol"), vm.getCode("Counter.sol:Counter:0.8.17"));
    }

    function testGetCodeByNameMultiVersion() public {
        assertEq(vm.getCode("Counter"), type(Counter).creationCode);
        require(keccak256(vm.getCode("Counter")) != keccak256(vm.getCode("Counter:0.8.18")), "Invalid artifact");
        assertEq(vm.getCode("Counter.sol"), vm.getCode("Counter:0.8.17"));
    }
}
