// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

/// @notice A contract used to test multi-profile compilation.
/// Different optimizer settings produce different bytecode.
contract MultiProfileContract {
    uint256 public count;

    function increment() public {
        count += 1;
    }

    function complexOp(uint256 a, uint256 b) public pure returns (uint256) {
        uint256 result = 0;
        for (uint256 i = 0; i < 10; i++) {
            result += a * b + i;
        }
        return result;
    }
}

/// @notice Tests vm.getCode with multiple compilation profiles.
/// Verifies that different profiles produce different bytecode and can be selected.
contract GetCodeMultiProfileTest is Test {
    function testGetCodeDifferentProfilesProduceDifferentBytecode() public {
        // Get bytecode for each profile
        bytes memory optimizedCode = vm.getCode("MultiProfileContract:optimized");
        bytes memory unoptimizedCode = vm.getCode("MultiProfileContract:unoptimized");

        // Both should return valid bytecode
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

    function testGetCodeByFullPathAndProfile() public {
        bytes memory code = vm.getCode("default/cheats/GetCodeMultiProfile.t.sol:MultiProfileContract:optimized");
        assertGt(code.length, 0, "should get bytecode with full path and profile");
    }

    function testDeployCodeWithDifferentProfiles() public {
        // Deploy the unoptimized version
        address unoptimizedAddr = vm.deployCode("MultiProfileContract:unoptimized");
        assertGt(unoptimizedAddr.code.length, 0, "should deploy unoptimized contract");

        // Deploy the optimized version
        address optimizedAddr = vm.deployCode("MultiProfileContract:optimized");
        assertGt(optimizedAddr.code.length, 0, "should deploy optimized contract");

        // Deployed code should differ
        assertNotEq(
            keccak256(unoptimizedAddr.code),
            keccak256(optimizedAddr.code),
            "deployed code should differ between profiles"
        );
    }

    function testGetDeployedCodeWithDifferentProfiles() public {
        // Get deployed (runtime) bytecode for each profile
        bytes memory optimizedDeployed = vm.getDeployedCode("MultiProfileContract:optimized");
        bytes memory unoptimizedDeployed = vm.getDeployedCode("MultiProfileContract:unoptimized");

        assertGt(optimizedDeployed.length, 0, "should get optimized deployed code");
        assertGt(unoptimizedDeployed.length, 0, "should get unoptimized deployed code");

        // Runtime bytecode should also differ
        assertNotEq(
            keccak256(optimizedDeployed), keccak256(unoptimizedDeployed), "deployed code should differ between profiles"
        );
    }

    function testGetCodeNonExistentProfileFails() public {
        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getCode("MultiProfileContract:nonexistent");
    }
}
