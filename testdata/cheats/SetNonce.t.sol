// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

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

    function testSetNonce() public {
        vm.setNonce(address(foo), 10);
        // makes sure working correctly after mutating nonce.
        foo.f();
        assertEq(vm.getNonce(address(foo)), 10);
        foo.f();
    }

    function testFailInvalidNonce() public {
        vm.setNonce(address(foo), 10);
        // set lower nonce should fail
        vm.setNonce(address(foo), 5);
    }
}
