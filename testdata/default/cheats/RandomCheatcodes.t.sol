// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RandomCheatcodesTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    int128 constant MIN = -170141183460469231731687303715884105728;
    int128 constant MAX = 170141183460469231731687303715884105727;

    function test_int128() public {
        VM._expectCheatcodeRevert("VM.randomInt: number of bits cannot exceed 256");
        int256 val = VM.randomInt(type(uint256).max);

        val = VM.randomInt(128);
        assertGe(val, MIN);
        assertLe(val, MAX);
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testReverttIf_int128() public {
        int256 val = VM.randomInt(128);
        VM.expectRevert("Error: a > b not satisfied [int]");
        require(val > MAX, "Error: a > b not satisfied [int]");
    }

    function test_address() public {
        address freshAddress = VM.randomAddress();
        assert(freshAddress != address(this));
        assert(freshAddress != address(VM));
    }

    function test_randomUintLimit() public {
        VM._expectCheatcodeRevert("VM.randomUint: number of bits cannot exceed 256");
        VM.randomUint(type(uint256).max);
    }

    function test_randomUints(uint256 x) public {
        x = VM.randomUint(0, 256);
        uint256 freshUint = VM.randomUint(x);

        assert(0 <= freshUint);
        if (x == 256) {
            assert(freshUint <= type(uint256).max);
        } else {
            assert(freshUint <= 2 ** x - 1);
        }
    }

    function test_randomSymbolicWord() public {
        uint256 freshUint192 = VM.randomUint(192);

        assert(0 <= freshUint192);
        assert(freshUint192 <= type(uint192).max);
    }
}

contract RandomBytesTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    bytes1 localByte;
    bytes localBytes;

    function manipSymbolicBytes(bytes memory b) public {
        uint256 middle = b.length / 2;
        b[middle] = hex"aa";
    }

    function test_symbolic_bytes_revert() public {
        VM._expectCheatcodeRevert();
        VM.randomBytes(type(uint256).max);
    }

    function test_symbolic_bytes_1() public {
        uint256 length = uint256(VM.randomUint(1, type(uint8).max));
        bytes memory freshBytes = VM.randomBytes(length);
        uint256 index = uint256(VM.randomUint(1));

        localByte = freshBytes[index];
        assertEq(freshBytes[index], localByte);
    }

    function test_symbolic_bytes_2() public {
        uint256 length = uint256(VM.randomUint(1, type(uint8).max));
        bytes memory freshBytes = VM.randomBytes(length);

        localBytes = freshBytes;
        assertEq(freshBytes, localBytes);
    }

    function test_symbolic_bytes_3() public {
        uint256 length = uint256(VM.randomUint(1, type(uint8).max));
        bytes memory freshBytes = VM.randomBytes(length);

        manipSymbolicBytes(freshBytes);
        assertEq(hex"aa", freshBytes[length / 2]);
    }

    function test_symbolic_bytes_length(uint8 l) public {
        VM.assume(0 < l);
        bytes memory freshBytes = VM.randomBytes(l);
        assertEq(freshBytes.length, l);
    }
}
