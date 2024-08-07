// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RandomUint is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRandomUint() public {
        vm.randomUint();
    }

    /// forge-config: default.fuzz.seed = 0x123
    function testRandomUintWithSeed() public {
        uint256 n = vm.randomUint();
        uint256 m = vm.randomUint();
        assertEq(n, m);
    }

    function testRandomUintWithoutSeed() public {
        uint256 n = vm.randomUint();
        uint256 m = vm.randomUint();
        vm.assertNotEq(n, m);
    }

    function testRandomUintRangeOverflow() public {
        vm.randomUint(0, uint256(int256(-1)));
    }

    function testRandomUintSame(uint256 val) public {
        uint256 rand = vm.randomUint(val, val);
        assertTrue(rand == val);
    }

    function testRandomUintRange(uint256 min, uint256 max) public {
        vm.assume(max >= min);
        uint256 rand = vm.randomUint(min, max);
        assertTrue(rand >= min, "rand >= min");
        assertTrue(rand <= max, "rand <= max");
    }

    /// forge-config: default.fuzz.seed = 0x123
    function testRandomUintRangeWithSeed(uint256 min, uint256 max) public {
        vm.assume(max >= min);
        uint256 n = vm.randomUint(min, max);
        assertTrue(n >= min, "n >= min");
        assertTrue(n <= max, "n <= max");

        uint256 m = vm.randomUint(min, max);
        assertEq(n, m);
    }

    function testRandomUintRangeWithoutSeed(uint256 min, uint256 max) public {
        vm.assume(max >= min);
        uint256 n = vm.randomUint(min, max);
        assertTrue(n >= min, "n >= min");
        assertTrue(n <= max, "n <= max");

        uint256 m = vm.randomUint(min, max);
        vm.assertNotEq(n, m);
    }
}
