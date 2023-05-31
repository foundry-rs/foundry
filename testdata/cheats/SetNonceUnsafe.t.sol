// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Foo {
    function f() external view returns (uint256) {
        return 1;
    }
}

contract SetNonceTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    Foo public foo;

    function setUp() public {
        foo = new Foo();
    }

    function testSetNonceUnsafe() public {
        cheats.setNonceUnsafe(address(foo), 10);
        // makes sure working correctly after mutating nonce.
        foo.f();
        assertEq(cheats.getNonce(address(foo)), 10);
        foo.f();
    }

    function testDoesNotFailDecreasingNonce() public {
        cheats.setNonce(address(foo), 10);
        cheats.setNonceUnsafe(address(foo), 5);
        assertEq(cheats.getNonce(address(foo)), 5);
    }
}
