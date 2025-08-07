// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Rlp is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testToRlp() public {
        bytes[] memory data = new bytes[](2);
        data[0] = hex"01";
        data[1] = hex"02";

        bytes memory rlp = vm.toRlp(data);

        // Assert the expected RLP encoding for [0x01, 0x02]
        // 0xc2 = list with 2 bytes total length
        // 0x01 = first byte
        // 0x02 = second byte
        assertEq(rlp, hex"c20102");
    }

    function testFromRlp() public {
        // RLP encoded [0x01, 0x02]
        bytes memory rlp = hex"c20102";

        bytes[] memory decoded = vm.fromRlp(rlp);
        assertEq(decoded.length, 2);
        assertEq(decoded[0], hex"01");
        assertEq(decoded[1], hex"02");
    }

    function testRoundTrip() public {
        bytes[] memory original = new bytes[](3);
        original[0] = hex"deadbeef";
        original[1] = hex"cafebabe";
        original[2] = hex"01020304";

        bytes memory rlp = vm.toRlp(original);
        bytes[] memory decoded = vm.fromRlp(rlp);

        assertEq(decoded.length, original.length);
        for (uint i = 0; i < original.length; i++) {
            assertEq(decoded[i], original[i]);
        }
    }
}
