// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract SeedTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSeedAffectsRandom() public {
        // Use a known seed
        uint256 seed = 123456789;
        vm.setSeed(seed);

        // Call a foundry cheatcode to get a random value (this depends on the integration)
        uint256 rand1 = uint256(vm.randomUint());

        // Reset the seed and verify the result is the same
        vm.setSeed(seed);
        uint256 rand2 = uint256(vm.randomUint());
        // If the seed is the same, the random value must be equal
        assertEq(rand1, rand2);
    }

    function testSeedChangesRandom() public {
        // Use one seed
        vm.setSeed(1);
        uint256 randA = uint256(vm.randomUint());

        // Use a different seed
        vm.setSeed(2);
        uint256 randB = uint256(vm.randomUint());

        // Values must be different
        assertTrue(randA != randB, "Random value must be different if seed is different");
    }
}
