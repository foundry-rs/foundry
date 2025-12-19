// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Issue12817Test is Test {
    function test_fuzz_randomUintRange(uint256 seed) public {
        vm.seed(seed);
        uint256 randomUint = vm.randomUint(0, 3);
        if (randomUint != 1) {
            revert("Random value was not 1");
        }
    }

    function test_fuzz_randomUint(uint256 seed) public {
        vm.seed(seed);
        uint256 randomUint = vm.randomUint() % 4;
        if (randomUint != 1) {
            revert("Random value was not 1");
        }
    }
}
