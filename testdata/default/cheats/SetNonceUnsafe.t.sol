// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Foo {
    function f() external view returns (uint256) {
        return 1;
    }
}

contract SetNonceTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
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
