// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Foo {}

contract GetNonceTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testGetNonce() public {
        uint64 nonce1 = cheats.getNonce(address(this));
        new Foo();
        new Foo();
        uint64 nonce2 = cheats.getNonce(address(this));
        assertEq(nonce1 + 2, nonce2);
    }
}
