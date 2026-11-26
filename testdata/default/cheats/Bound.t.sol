// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract BoundTest is Test {
    function testBoundReachesEveryOtherValue() public {
        bool[4] memory hit;
        for (uint256 i = 0; i < 200; i++) {
            uint256 value = vm.bound(uint256(1), 0, 3);
            assertTrue(value <= 3);
            assertTrue(value != 1);
            hit[value] = true;
        }
        assertTrue(hit[0] && hit[2] && hit[3]);
    }

    function testBoundSignedReachesEveryOtherValue() public {
        bool[5] memory hit;
        for (uint256 i = 0; i < 200; i++) {
            int256 value = vm.bound(int256(0), -2, 2);
            assertTrue(value >= -2 && value <= 2);
            assertTrue(value != 0);
            hit[uint256(value + 2)] = true;
        }
        assertTrue(hit[0] && hit[1] && hit[3] && hit[4]);
    }

    /// Values beyond 128 bits are reachable on full-width ranges.
    function testBoundReachesWideValues() public {
        bool high;
        bool highSigned;
        bool lowSigned;
        for (uint256 i = 0; i < 100; i++) {
            high = high || vm.bound(uint256(0), 0, type(uint256).max) > type(uint128).max;
            int256 signed = vm.bound(int256(0), type(int256).min, type(int256).max);
            highSigned = highSigned || signed > int256(uint256(type(uint128).max));
            lowSigned = lowSigned || signed < -int256(uint256(type(uint128).max));
        }
        assertTrue(high);
        assertTrue(highSigned);
        assertTrue(lowSigned);
    }

    function testBoundRejectsCurrentOutsideRange() public {
        vm._expectCheatcodeRevert("vm.bound: cannot bound 4 in [0, 3] range");
        vm.bound(uint256(4), 0, 3);
    }

    function testBoundRejectsRangeWithoutAnotherValue() public {
        vm._expectCheatcodeRevert("vm.bound: cannot bound 3 in [3, 3] range");
        vm.bound(uint256(3), 3, 3);
    }
}
