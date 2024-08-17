// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RandomUint is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRandomUint() public {
        vm.randomUint();
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
}
