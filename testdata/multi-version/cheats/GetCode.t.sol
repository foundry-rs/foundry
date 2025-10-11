// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity =0.8.18;

import "utils/Test.sol";
import "../Counter.sol";

contract GetCodeTest is Test {
    function testGetCodeMultiVersion() public {
        assertEq(vm.getCode("Counter.sol"), type(Counter).creationCode);
        require(
            keccak256(vm.getCode("Counter.sol")) != keccak256(vm.getCode("Counter.sol:Counter:0.8.17")),
            "Invalid artifact"
        );
        assertEq(vm.getCode("Counter.sol"), vm.getCode("Counter.sol:Counter:0.8.18"));
    }

    function testGetCodeByNameMultiVersion() public {
        assertEq(vm.getCode("Counter"), type(Counter).creationCode);
        require(keccak256(vm.getCode("Counter")) != keccak256(vm.getCode("Counter:0.8.17")), "Invalid artifact");
        assertEq(vm.getCode("Counter"), vm.getCode("Counter:0.8.18"));
    }
}
