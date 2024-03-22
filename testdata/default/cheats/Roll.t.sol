// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RollTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRoll() public {
        vm.roll(10);
        assertEq(block.number, 10, "roll failed");
    }

    function testRollFuzzed(uint128 jump) public {
        uint256 pre = block.number;
        vm.roll(block.number + jump);
        assertEq(block.number, pre + jump, "roll failed");
    }

    function testRollHash() public {
        assertEq(blockhash(block.number), 0x0, "initial block hash is incorrect");

        vm.roll(5);
        bytes32 hash = blockhash(5);
        assertTrue(blockhash(4) != 0x0, "new block hash is incorrect");

        vm.roll(10);
        assertTrue(blockhash(5) != blockhash(10), "block hash collision");

        vm.roll(5);
        assertEq(blockhash(5), hash, "block 5 changed hash");
    }
}
