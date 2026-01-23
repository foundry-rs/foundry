// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract ParisContract {
    uint256 public value = 42;
}

contract GetCodeProfileTest is Test {
    function testGetCodeByProfile() public {
        // Get code for a contract compiled with the paris profile
        bytes memory code = vm.getCode("paris/cheats/GetCodeProfile.t.sol:ParisContract:paris");
        assertGt(code.length, 0, "should get bytecode for paris profile");
        assertEq(code, type(ParisContract).creationCode, "bytecode should match");
    }

    function testGetCodeByContractNameAndProfile() public {
        // Get code by contract name and profile
        bytes memory code = vm.getCode("ParisContract:paris");
        assertGt(code.length, 0, "should get bytecode for paris profile by name");
        assertEq(code, type(ParisContract).creationCode, "bytecode should match");
    }

    function testGetCodeWrongProfile() public {
        // Trying to get code with non-existent profile should fail
        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getCode("ParisContract:nonexistent");
    }
}
