// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Foo {
    function f() external view returns (uint256) {
        return 1;
    }
}

contract SetNonceTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);
    Foo public foo;

    function setUp() public {
        foo = new Foo();
    }

    function testSetNonceUnsafe() public {
        VM.setNonceUnsafe(address(foo), 10);
        // makes sure working correctly after mutating nonce.
        foo.f();
        assertEq(VM.getNonce(address(foo)), 10);
        foo.f();
    }

    function testDoesNotFailDecreasingNonce() public {
        VM.setNonce(address(foo), 10);
        VM.setNonceUnsafe(address(foo), 5);
        assertEq(VM.getNonce(address(foo)), 5);
    }
}
