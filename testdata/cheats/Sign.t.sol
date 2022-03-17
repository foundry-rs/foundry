// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract SignTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testSignDigest(uint248 pk, bytes32 digest) public {
        cheats.assume(pk != 0);

        (uint8 v, bytes32 r, bytes32 s) = cheats.sign(pk, digest);
        address expected = cheats.addr(pk);
        address actual = ecrecover(digest, v, r, s);

        assertEq(actual, expected, "digest signer did not match");
    }

    function testSignMessage(uint248 pk, bytes memory message) public {
        testSignDigest(pk, keccak256(message));
    }
}
