// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// @notice A contract used to test profile-based artifact selection.
contract ParisProfileContract {
    uint256 public value = 42;

    function getValue() public view returns (uint256) {
        return value;
    }
}

/// @notice Tests vm.getCode with compilation profile parameter.
/// The paris directory uses the "paris" profile which has evm_version = "paris".
/// This test verifies that the profile can be used to select artifacts.
contract GetCodeProfileTest is Test {
    function testGetCodeByProfile() public {
        // Get code for a contract compiled with the paris profile
        // The format is: path:ContractName:profile
        bytes memory code = vm.getCode("paris/cheats/GetCodeProfile.t.sol:ParisProfileContract:paris");
        assertGt(code.length, 0, "should get bytecode for paris profile");
        assertEq(code, type(ParisProfileContract).creationCode, "bytecode should match");
    }

    function testGetCodeByContractNameAndProfile() public {
        // Get code by contract name and profile
        bytes memory code = vm.getCode("ParisProfileContract:paris");
        assertGt(code.length, 0, "should get bytecode for paris profile by name");
        assertEq(code, type(ParisProfileContract).creationCode, "bytecode should match");
    }

    function testGetDeployedCodeByProfile() public {
        // Get deployed code by profile
        bytes memory code = vm.getDeployedCode("ParisProfileContract:paris");
        assertGt(code.length, 0, "should get deployed bytecode for paris profile");
    }

    function testDeployCodeByProfile() public {
        // Deploy using profile
        address deployed = vm.deployCode("ParisProfileContract:paris");
        assertGt(deployed.code.length, 0, "should deploy contract");
    }

    function testGetCodeWrongProfile() public {
        // Trying to get code with non-existent profile should fail
        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getCode("ParisProfileContract:nonexistent");
    }

    function testGetCodeDefaultProfileFails() public {
        // The paris contract is ONLY compiled with paris profile,
        // so requesting "default" profile should fail
        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getCode("ParisProfileContract:default");
    }
}
