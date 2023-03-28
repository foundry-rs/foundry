// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract GetMemoryTest is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function testGetMemory() public {
        assertEq(vm.getMemory(0, 31), abi.encodePacked(bytes32(0)));

        assembly {
            mstore(0, 0x4141414141414141414141414141414141414141414141414141414141414141)
            mstore(0x20, 0xbabababababababababababababababababababababababababababababababa)
        }
        bytes memory mem1 = vm.getMemory(0, 12);
        bytes memory mem2 = vm.getMemory(0x20, 0x3f);
        
        assertEq(mem1.length, 13);
        assertEq(mem2.length, 32);

        assertEq(mem1, hex"41414141414141414141414141");
        assertEq(mem2, hex"babababababababababababababababababababababababababababababababa");

        bytes memory mem3 = vm.getMemory(0x60, 0x7f);
        assertEq(mem3.length, 32);
        assertEq(mem3, abi.encodePacked(bytes32(0)));
    }

    function testGetMemoryAsString() public {
        string memory mem = vm.getMemoryFormattedAsString(10, 20);
        assertEq(mem, " 0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31\n                                00 00 00 00 00 00 00 00 00 00 00                                   0x00 (0)\n");

        assembly {
            mstore8(10, 0x41)
            mstore8(20, 0xba)
        }

        assertEq(vm.getMemoryFormattedAsString(5, 20), " 0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31\n                 00 00 00 00 00 41 00 00 00 00 00 00 00 00 00 ba                                   0x00 (0)\n");
    }

    function testGetMemoryFormatted() public {
        Cheats.FormattedMemory memory mem = vm.getMemoryFormatted(0, 0x40);
        assertEq(mem.header, " 0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31");
        assertEq(mem.words.length, 3);
        assertEq(mem.words[0], "00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00  0x00 (0)");
        assertEq(mem.words[2], "00                                                                                               0x40 (64)");
    }

    // Reverts
    function testFailGetMemoryStartOverEndIndex() public {
        vm.getMemory(20, 12);
    }

    function testFailGetMemoryStartIndexTooHigh() public {
        vm.getMemory(300, 301);
    }

    function testFailGetMemoryEndIndexTooHigh() public {
        vm.getMemory(20, 300);
    }

    // // Let's make sure our error messages make sense
    // function testRevertsErrorMessages() public {
    //     // This doesn't work, the error message and the expected revert message don't match
    //     // maybe we don't format the error message correctly in `check_format_memory_inputs()` ?
    //     vm.expectRevert("Error getMemory: invalid parameters: start (20) must be <= end (12)");
    //     vm.getMemory(20, 12);
    // }
}
