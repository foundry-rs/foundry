// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RandomUint is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // All tests use `>=` and `<=` to verify that ranges are inclusive and that
    // a value of zero may be generated.
    function testRandomUint() public {
        uint256 rand = vm.randomUint();
        assertTrue(rand >= 0);
    }

    function testRandomUint(uint256 min, uint256 max) public {
        vm.assume(max >= min);
        uint256 rand = vm.randomUint(min, max);
        assertTrue(rand >= min, "rand >= min");
        assertTrue(rand <= max, "rand <= max");
    }

    function testRandomUint(uint256 val) public {
        uint256 rand = vm.randomUint(val, val);
        assertTrue(rand == val);
    }

    function testRandomAddress() public {
        address rand = vm.randomAddress();
        assertTrue(rand >= address(0));
    }
}
