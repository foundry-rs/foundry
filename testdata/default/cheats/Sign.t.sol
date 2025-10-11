// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SignTest is Test {
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

    /// secp256k1 subgroup order n
    function _secp256k1Order() internal pure returns (uint256) {
        return 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;
    }

    function testsignWithNonceUnsafeDigestDifferentNonces(uint248 pk, bytes32 digest) public {
        vm.assume(pk != 0);
        uint256 n1 = 123;
        uint256 n2 = 456;
        vm.assume(n1 != 0 && n2 != 0 && n1 != n2);
        (uint8 v1, bytes32 r1, bytes32 s1) = vm.signWithNonceUnsafe(pk, digest, n1);
        (uint8 v2, bytes32 r2, bytes32 s2) = vm.signWithNonceUnsafe(pk, digest, n2);
        assertTrue(r1 != r2 || s1 != s2, "signatures should differ for different nonces");
        address expected = vm.addr(pk);
        assertEq(ecrecover(digest, v1, r1, s1), expected, "recover for nonce n1 failed");
        assertEq(ecrecover(digest, v2, r2, s2), expected, "recover for nonce n2 failed");
    }

    function testsignWithNonceUnsafeDigestSameNonceDeterministic(uint248 pk, bytes32 digest) public {
        vm.assume(pk != 0);
        uint256 n = 777;
        vm.assume(n != 0);
        (uint8 v1, bytes32 r1, bytes32 s1) = vm.signWithNonceUnsafe(pk, digest, n);
        (uint8 v2, bytes32 r2, bytes32 s2) = vm.signWithNonceUnsafe(pk, digest, n);
        assertEq(v1, v2, "v should match");
        assertEq(r1, r2, "r should match");
        assertEq(s1, s2, "s should match");
        address expected = vm.addr(pk);
        assertEq(ecrecover(digest, v1, r1, s1), expected, "recover failed");
    }

    function testsignWithNonceUnsafeInvalidNoncesRevert() public {
        uint256 pk = 1;
        bytes32 digest = 0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb;
        (bool ok, bytes memory data) =
            HEVM_ADDRESS.call(abi.encodeWithSelector(Vm.signWithNonceUnsafe.selector, pk, digest, 0));
        assertTrue(!ok, "expected revert on nonce=0");
        assertEq(_revertString(data), "vm.signWithNonceUnsafe: nonce cannot be 0");
        uint256 n = _secp256k1Order();
        (ok, data) = HEVM_ADDRESS.call(abi.encodeWithSelector(Vm.signWithNonceUnsafe.selector, pk, digest, n));
        assertTrue(!ok, "expected revert on nonce >= n");
        assertEq(_revertString(data), "vm.signWithNonceUnsafe: invalid nonce scalar");
    }

    /// Decode revert payload
    /// by stripping the 4-byte selector and ABI-decoding the tail as `string`.
    function _revertString(bytes memory data) internal pure returns (string memory) {
        if (data.length < 4) return "";
        // copy data[4:] into a new bytes
        bytes memory tail = new bytes(data.length - 4);
        for (uint256 i = 0; i < tail.length; i++) {
            tail[i] = data[i + 4];
        }
        return abi.decode(tail, (string));
    }
}
