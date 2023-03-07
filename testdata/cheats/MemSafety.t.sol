// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract MemSafetyTest is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    /// @dev Tests that writing to memory within the range given to `allowMemoryWrites`
    ///      will not cause the test to fail while using the `MSTORE` opcode.
    function testAllowMemoryWrites_MSTORE() public {
        vm.allowMemoryWrites(0x80, 0xA0);
        assembly { mstore(0x80, 0xc0ffee) }
    }

    /// @dev Tests that writing to memory within the ranges given to `allowMemoryWrites`
    ///      will not cause the test to fail while using the `MSTORE` opcode.
    function testAllowMemoryWrites_multiRange_MSTORE() public {
        vm.allowMemoryWrites(0x80, 0x100);
        vm.allowMemoryWrites(0x120, 0x140);
        assembly {
            mstore(0x80, 0xc0ffee)
            mstore(0x120, 0xbadf00d)
        }
    }

    /// @dev Tests that writing to memory within the range given to `allowMemoryWrites`
    ///      will not cause the test to fail while using the `MSTORE8` opcode.
    function testAllowMemoryWrites_MSTORE8() public {
        vm.allowMemoryWrites(0x80, 0x81);
        assembly { mstore8(0x80, 0xFF) }
    }

    /// @dev Tests that writing to memory within the ranges given to `allowMemoryWrites`
    ///      will not cause the test to fail while using the `MSTORE8` opcode.
    function testAllowMemoryWrites_multiRange_MSTORE8() public {
        vm.allowMemoryWrites(0x80, 0x100);
        vm.allowMemoryWrites(0x120, 0x121);
        assembly {
            mstore8(0x80, 0xFF)
            mstore8(0x120, 0xFF)
        }
    }

    /// @dev Tests that writing to memory before the range given to `allowMemoryWrites`
    ///      will cause the test to fail while using the `MSTORE` opcode.
    function testFailAllowMemoryWrites_MSTORELow() public {
        vm.allowMemoryWrites(0x80, 0xA0);
        assembly { mstore(0x60, 0xc0ffee) }
    }

    /// @dev Tests that writing to memory after the range given to `allowMemoryWrites`
    ///      will cause the test to fail while using the `MSTORE` opcode.
    function testFailAllowMemoryWrites_MSTOREHigh() public {
        vm.allowMemoryWrites(0x80, 0xA0);
        assembly { mstore(0xA0, 0xc0ffee) }
    }

    /// @dev Tests that writing to memory before the range given to `allowMemoryWrites`
    ///      will cause the test to fail while using the `MSTORE8` opcode.
    function testFailAllowMemoryWrites_MSTORE8Low() public {
        vm.allowMemoryWrites(0x80, 0x81);
        assembly { mstore8(0x60, 0xFF) }
    }

    /// @dev Tests that writing to memory after the range given to `allowMemoryWrites`
    ///      will cause the test to fail while using the `MSTORE8` opcode.
    function testFailAllowMemoryWrites_MSTORE8High() public {
        vm.allowMemoryWrites(0x80, 0x81);
        assembly { mstore8(0x81, 0xFF) }
    }

    /// @dev Tests that the `allowMemoryWrites` cheatcode respects context depth while
    ///      using the `MSTORE` opcode.
    function testAllowMemoryWrites_MSTORERespectsDepth() public {
        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.allowMemoryWrites(0x80, 0x100);

        // Should not revert- the `allowMemoryWrites` cheatcode operates at a
        // per-depth level.
        new SubContext().doMstore(0x120, 0xc0ffee);
    }

    /// @dev Tests that the `allowMemoryWrites` cheatcode respects context depth while
    ///      using the `MSTORE8` opcode.
    function testAllowMemoryWrites_MSTORE8RespectsDepth() public {
        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.allowMemoryWrites(0x80, 0x100);

        // Should not revert- the `allowMemoryWrites` cheatcode operates at a
        // per-depth level.
        new SubContext().doMstore8(0x120, 0xFF);
    }
}

/// @dev A simple contract to help ensure that the `allowMemoryWrites` cheatcode
///      respects context depth.
contract SubContext {
    function doMstore(uint256 offset, uint256 val) external {
        assembly { mstore(offset, val) }
    }

    function doMstore8(uint256 offset, uint8 val) external {
        assembly { mstore8(offset, val) }
    }
}
