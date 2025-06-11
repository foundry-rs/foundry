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

        uint256 rand3 = uint256(vm.randomUint());
        // If the seed is the same, the random value must be equal
        assertEq(rand1, rand2);
        assertTrue(rand1 != rand3);
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

    function testSeedAffectsShuffle() public {
        // Use a known seed
        uint256 seed = 123456789;
        vm.setSeed(seed);

        // Create two identical arrays
        uint256[] memory array1 = new uint256[](5);
        uint256[] memory array2 = new uint256[](5);
        for (uint256 i = 0; i < 5; i++) {
            array1[i] = i;
            array2[i] = i;
        }

        // Shuffle both arrays with the same seed
        array1 = vm.shuffle(array1);
        vm.setSeed(seed); // Reset the seed to get the same shuffle pattern
        array2 = vm.shuffle(array2);

        // Compare elements - they should be identical after shuffle
        for (uint256 i = 0; i < array1.length; i++) {
            assertEq(array1[i], array2[i], "Arrays should be identical with same seed");
        }
    }

    function testDifferentSeedsProduceDifferentShuffles() public {
        // Create the initial array
        uint256[] memory array1 = new uint256[](5);
        uint256[] memory array2 = new uint256[](5);
        for (uint256 i = 0; i < 5; i++) {
            array1[i] = i;
            array2[i] = i;
        }

        // Use first seed
        vm.setSeed(1);
        array1 = vm.shuffle(array1);

        // Use second seed
        vm.setSeed(2);
        array2 = vm.shuffle(array2);

        // Arrays should be different (we'll check at least one difference exists)
        bool foundDifference = false;
        for (uint256 i = 0; i < array1.length; i++) {
            if (array1[i] != array2[i]) {
                foundDifference = true;
                break;
            }
        }
        assertTrue(foundDifference, "Arrays should be different with different seeds");
    }
}
