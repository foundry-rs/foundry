// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract SignTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSignDigest(uint248 pk, bytes32 digest) public {
        vm.assume(pk != 0);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        address expected = vm.addr(pk);
        address actual = ecrecover(digest, v, r, s);
        assertEq(actual, expected, "digest signer did not match");
    }

    function testSignCompactDigest(uint248 pk, bytes32 digest) public {
        vm.assume(pk != 0);

        (bytes32 r, bytes32 vs) = vm.signCompact(pk, digest);

        // Extract `s` from `vs`.
        // Shift left by 1 bit to clear the leftmost bit, then shift right by 1 bit to restore the original position.
        // This effectively clears the leftmost bit of `vs`, giving us `s`.
        bytes32 s = bytes32((uint256(vs) << 1) >> 1);

        // Extract `v` from `vs`.
        // We shift `vs` right by 255 bits to isolate the leftmost bit.
        // Converting this to uint8 gives us the parity bit (0 or 1).
        // Adding 27 converts this parity bit to the correct `v` value (27 or 28).
        uint8 v = uint8(uint256(vs) >> 255) + 27;

        address expected = vm.addr(pk);
        address actual = ecrecover(digest, v, r, s);
        assertEq(actual, expected, "digest signer did not match");
    }

    function testSignMessage(uint248 pk, bytes memory message) public {
        testSignDigest(pk, keccak256(message));
    }

    function testSignCompactMessage(uint248 pk, bytes memory message) public {
        testSignCompactDigest(pk, keccak256(message));
    }
}
