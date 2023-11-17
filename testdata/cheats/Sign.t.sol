// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract SignTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSignDigest(uint248 pk, bytes32 digest) public {
        vm.assume(pk != 0);

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        address expected = vm.addr(pk);
        address actual = ecrecover(digest, v, r, s);

        assertEq(actual, expected, "digest signer did not match");
    }

    function testSignMessage(uint248 pk, bytes memory message) public {
        testSignDigest(pk, keccak256(message));
    }
}
