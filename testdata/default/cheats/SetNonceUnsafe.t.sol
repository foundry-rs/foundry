// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Foo {
    function f() external view returns (uint256) {
        return 1;
    }
}

contract SetNonceTest is Test {
    Foo public foo;

    function setUp() public {
        foo = new Foo();
    }

    function testSetNonceUnsafe() public {
        vm.setNonceUnsafe(address(foo), 10);
        // makes sure working correctly after mutating nonce.
        foo.f();
        assertEq(vm.getNonce(address(foo)), 10);
        foo.f();
    }

    function testDoesNotFailDecreasingNonce() public {
        vm.setNonce(address(foo), 10);
        vm.setNonceUnsafe(address(foo), 5);
        assertEq(vm.getNonce(address(foo)), 5);
    }
}
