// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// @notice Tests vm.getCode with profile parameter parsing.
/// This test verifies that the profile parameter is correctly parsed and used
/// for artifact selection. Since we can't easily compile the same contract
/// with multiple profiles in the test setup, we test the parsing behavior.
contract GetCodeMultiProfileTest is Test {
    function testGetCodeByProfileParsing() public {
        // Test that profile parsing works - the "default" profile should work
        // for contracts compiled with default settings
        bytes memory code = vm.getCode("multi-profile/Counter.sol:Counter:default");
        assertGt(code.length, 0, "should get bytecode with default profile");
    }

    function testGetCodeProfileNotFound() public {
        // Non-existent profile should fail with appropriate error
        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getCode("Counter:nonexistent_profile");
    }

    function testGetCodeByContractNameAndProfile() public {
        // Test the ContractName:profile format with default profile
        bytes memory code = vm.getCode("Counter:default");
        assertGt(code.length, 0, "should get bytecode by name and profile");
    }

    function testGetCodeProfileWithFullPath() public {
        // Test with full path format
        bytes memory code1 = vm.getCode("multi-profile/Counter.sol:Counter:default");
        bytes memory code2 = vm.getCode("Counter:default");

        // Both should return the same bytecode
        assertEq(keccak256(code1), keccak256(code2), "full path and name should return same bytecode");
    }

    function testDeployCodeWithProfile() public {
        // Deploy using the default profile
        address addr = vm.deployCode("Counter:default");
        assertGt(addr.code.length, 0, "should deploy contract with profile");
    }

    function testGetDeployedCodeWithProfile() public {
        // Get deployed (runtime) bytecode with profile
        bytes memory deployed = vm.getDeployedCode("Counter:default");
        assertGt(deployed.length, 0, "should get deployed code with profile");
    }
}
