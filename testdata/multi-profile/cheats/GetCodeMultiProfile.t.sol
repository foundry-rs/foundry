// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// @notice Tests vm.getCode with multiple compilation profiles.
/// This test verifies that:
/// 1. Different profiles produce different bytecode (due to optimizer settings)
/// 2. vm.getCode can correctly select bytecode by profile name
/// 3. The feature doesn't regress existing version-based selection
contract GetCodeMultiProfileTest is Test {
    function testGetCodeByProfileReturnsCorrectBytecode() public {
        // Get bytecode for each profile
        bytes memory defaultCode = vm.getCode("multi-profile/Counter.sol:Counter:default");
        bytes memory optimizedCode = vm.getCode("multi-profile/Counter.sol:Counter:optimized");
        bytes memory unoptimizedCode = vm.getCode("multi-profile/Counter.sol:Counter:unoptimized");

        // All should return valid bytecode
        assertGt(defaultCode.length, 0, "default profile should have bytecode");
        assertGt(optimizedCode.length, 0, "optimized profile should have bytecode");
        assertGt(unoptimizedCode.length, 0, "unoptimized profile should have bytecode");

        // Optimized vs unoptimized should produce different bytecode
        // (optimizer=true with 10000 runs vs optimizer=false)
        assertNotEq(
            keccak256(optimizedCode),
            keccak256(unoptimizedCode),
            "optimized and unoptimized profiles should produce different bytecode"
        );
    }

    function testGetCodeByContractNameAndProfile() public {
        // Test the ContractName:profile format
        bytes memory optimizedCode = vm.getCode("Counter:optimized");
        bytes memory unoptimizedCode = vm.getCode("Counter:unoptimized");

        assertGt(optimizedCode.length, 0, "should get optimized bytecode by name");
        assertGt(unoptimizedCode.length, 0, "should get unoptimized bytecode by name");

        // Should be different due to optimizer settings
        assertNotEq(
            keccak256(optimizedCode), keccak256(unoptimizedCode), "different profiles should produce different bytecode"
        );
    }

    function testGetCodeProfileNotFound() public {
        // Non-existent profile should fail
        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getCode("Counter:nonexistent_profile");
    }

    function testGetCodeProfileWithFullPath() public {
        // Test with full path format
        bytes memory code1 = vm.getCode("multi-profile/Counter.sol:Counter:optimized");
        bytes memory code2 = vm.getCode("Counter:optimized");

        // Both should return the same bytecode
        assertEq(keccak256(code1), keccak256(code2), "full path and name should return same bytecode");
    }

    function testDeployCodeWithProfile() public {
        // Deploy the unoptimized version
        address unoptimizedAddr = vm.deployCode("Counter:unoptimized");
        assertGt(unoptimizedAddr.code.length, 0, "should deploy unoptimized contract");

        // Deploy the optimized version
        address optimizedAddr = vm.deployCode("Counter:optimized");
        assertGt(optimizedAddr.code.length, 0, "should deploy optimized contract");

        // Deployed code should differ
        assertNotEq(
            keccak256(unoptimizedAddr.code),
            keccak256(optimizedAddr.code),
            "deployed code should differ between profiles"
        );
    }

    function testGetDeployedCodeWithProfile() public {
        // Get deployed (runtime) bytecode for each profile
        bytes memory optimizedDeployed = vm.getDeployedCode("Counter:optimized");
        bytes memory unoptimizedDeployed = vm.getDeployedCode("Counter:unoptimized");

        assertGt(optimizedDeployed.length, 0, "should get optimized deployed code");
        assertGt(unoptimizedDeployed.length, 0, "should get unoptimized deployed code");

        // Runtime bytecode should also differ
        assertNotEq(
            keccak256(optimizedDeployed), keccak256(unoptimizedDeployed), "deployed code should differ between profiles"
        );
    }
}
